use super::open::import_at_raw_ppi;
use super::paths::{extension, file_name, join, list_files, list_images, read_file, sanitize, stem, write_file};
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
    collect_inputs(p, cmd, output, false)
}

/// Collect folder inputs with Batch's explicit unsupported-file policy; arrays keep their order.
fn collect_inputs(p: &Value, cmd: &str, output: &str, include_non_images: bool) -> Result<Vec<BatchInput>> {
    let recurse = match p.get("includeSubfolders") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"includeSubfolders\" must be true or false".into() }),
    };
    match p.get("input").or_else(|| p.get("files")) {
        Some(Value::String(dir)) if recurse => list_files_tree(dir, output, cmd, include_non_images),
        Some(Value::String(dir)) if include_non_images => {
            Ok(list_files(dir, true)?.into_iter().map(|path| BatchInput { path, rel_dir: String::new() }).collect())
        }
        _ => Ok(batch_inputs(p, cmd)?.into_iter().map(|path| BatchInput { path, rel_dir: String::new() }).collect()),
    }
}

/// The selected files under `dir` and its subfolders: each folder's files sorted by name, then its
/// subfolders in name order (depth first). Bounded by [`MAX_SUBFOLDER_DEPTH`] and
/// [`MAX_SUBFOLDER_FILES`]; a folder reached twice (a symlink loop) is walked once, and `skip`
/// (the output folder) is left out.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn list_files_tree(dir: &str, skip: &str, cmd: &str, include_non_images: bool) -> Result<Vec<BatchInput>> {
    use std::path::{Path, PathBuf};
    let canon = |p: &Path| std::fs::canonicalize(p).map_err(|error| EngineError::Other(format!("{}: {error}", p.display())));
    let root = Path::new(dir);
    let root_canon = canon(root)?;
    let skip = if skip.is_empty() {
        None
    } else {
        match std::fs::canonicalize(skip) {
            Ok(path) => Some(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(EngineError::Other(format!("{skip}: {error}"))),
        }
    };
    let mut seen = std::collections::HashSet::new();
    seen.insert(root_canon);
    let mut out = Vec::new();
    // Depth first with an explicit stack (no recursion): (folder, relative folder, depth).
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((folder, rel, depth)) = stack.pop() {
        let shown = folder.to_string_lossy().into_owned();
        // A reached unreadable folder fails before any output is written.
        let files = list_files(&shown, include_non_images)?;
        if out.len().saturating_add(files.len()) > MAX_SUBFOLDER_FILES {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("{dir} holds more than {MAX_SUBFOLDER_FILES} files with its subfolders") });
        }
        out.extend(files.into_iter().map(|path| BatchInput { path, rel_dir: rel.clone() }));
        if depth >= MAX_SUBFOLDER_DEPTH {
            continue;
        }
        let rd = std::fs::read_dir(&folder).map_err(|error| EngineError::Other(format!("{shown}: {error}")))?;
        let mut subs: Vec<(String, PathBuf)> = Vec::new();
        for entry in rd {
            let path = entry.map_err(|error| EngineError::Other(format!("{shown}: {error}")))?.path();
            let metadata = match std::fs::metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue, // Broken links are not inputs.
                Err(error) => return Err(EngineError::Other(format!("{}: {error}", path.display()))),
            };
            if metadata.is_dir()
                && let Some(name) = path.file_name()
            {
                subs.push((name.to_string_lossy().into_owned(), path));
            }
        }
        subs.sort();
        // Pushed in reverse so the stack pops them in name order.
        for (name, sub) in subs.into_iter().rev() {
            let c = canon(&sub)?;
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
pub(crate) fn list_files_tree(dir: &str, _skip: &str, _cmd: &str, _include_non_images: bool) -> Result<Vec<BatchInput>> {
    Err(EngineError::Other(format!("cannot list {dir}: no file system on the web")))
}

/// Image-only tree traversal shared with native Batch consumers.
pub fn list_images_tree(dir: &str, skip: &str, cmd: &str) -> Result<Vec<BatchInput>> {
    list_files_tree(dir, skip, cmd, false)
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
    let no_writer = photocraft_formats::from_extension(&ext).is_some_and(|f| f.capabilities().write != photocraft_formats::Availability::Available);
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
    for f in photocraft_formats::Format::ALL.into_iter().filter(|f| {
        f.capabilities().read == photocraft_formats::Availability::Available && f.capabilities().write != photocraft_formats::Availability::Available
    }) {
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

/// A per-input failure retaining the complete response from nested work.
struct FileFailure {
    error: EngineError,
    result: Option<Value>,
}

impl From<EngineError> for FileFailure {
    fn from(error: EngineError) -> Self {
        Self { error, result: None }
    }
}

impl FileFailure {
    /// Ordinary failures keep their shape; nested failures also keep their results.
    fn value(&self, key: &str, input: &str) -> Value {
        let mut failure = json!({"error": self.error.to_string()});
        failure[key] = json!(input);
        // Marks a nesting or playback-budget stop, which an enclosing batch step must not swallow.
        if matches!(self.error, EngineError::BatchLimit(_)) {
            failure["limit"] = json!(true);
        }
        if let Some(result) = &self.result {
            failure["result"] = result.clone();
        }
        failure
    }
}

/// Run one source through its action and output pipeline while finalizing only that document's owned history scope.
fn process_outputs<'a>(
    s: &Session,
    outputs: impl IntoIterator<Item = (&'a str, String)>,
    save: SaveOpts,
    f: &dyn Fn(&mut Session, &str) -> Result<bool>,
) -> Value {
    // The scratch sessions have default preferences; a raw opens at the user's Raw Defaults.
    let raw_ppi = s.prefs().raw_defaults.resolution;
    let action_prefs = s.prefs().actions.clone();
    let authorize = s.authorize;
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut written = OutputClaims::default();
    let run = |scratch: &mut Session, out: &str| {
        // Nested commands answer to the initiating session's authorization hook.
        scratch.authorize = authorize;
        scratch.prefs.edit(|prefs| prefs.actions = action_prefs.clone());
        f(scratch, out).map_err(FileFailure::from)
    };
    for (path, out) in outputs {
        let r = process_one(raw_ppi, path, &out, save, ProcessOptions { format: None, written: Some(&written), ctx: None, input: None }, &run);
        match r {
            Ok(()) => {
                written.record(&out, path);
                files.push(out);
            }
            Err(error) => errors.push(error.value("file", path)),
        }
    }
    json!({"files": files, "errors": errors})
}

type PrepareInput<'a> = &'a dyn Fn(&mut Session, &str) -> Result<()>;

/// Process the current in-memory images in scratch sessions: unsaved edits are included and
/// resizing never touches the open documents' pixels, selection, history or save state.
/// Untitled names use the shared filename sanitizer before becoming output stems.
pub(super) fn process_document_copies(
    s: &Session,
    documents: &[&crate::DocState],
    output: &str,
    format: &str,
    save: SaveOpts,
    f: &dyn Fn(&mut Session, &str) -> Result<bool>,
) -> Value {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut written = OutputClaims::default();
    for state in documents {
        // Export resolves stored links against this source, including non-active documents.
        let _document = crate::doc_path::scope(state.path.as_deref().map(std::path::Path::new));
        // An untitled document's name is a label, not a path: keep it as one portable component.
        let path = state.path.clone().unwrap_or_else(|| format!("{}.psd", sanitize(&state.doc.name)));
        let ext = if format == "same" { same_format(&path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
        let out = join(output, &format!("{}.{ext}", stem(&path)));
        // Copies must not overwrite any open source file, including aliases of its path.
        if s.documents().iter().filter_map(|doc| doc.path.as_deref()).any(|source| super::paths::same_destination(source, &out)) {
            errors.push(json!({"file": path, "error": format!("output would replace an open source file: {out}; choose a different output folder")}));
            continue;
        }
        let prepare = |scratch: &mut Session, _: &str| {
            scratch.authorize = s.authorize;
            scratch.prefs.edit(|prefs| *prefs = s.prefs().clone());
            scratch.add_document(state.doc.as_ref().clone(), state.path.clone());
            // The moved selection belongs to the displayed image; commit it only in the copy.
            if let Some(floating) = crate::float_cmds::floating(state).cloned() {
                let copy = scratch.active_mut().ok_or(EngineError::NoDocument)?;
                copy.floating = Some(crate::float_cmds::Floating { revision: copy.revision, ..floating });
                crate::float_cmds::drop_floating(scratch)?;
            }
            Ok(())
        };
        let notices = std::cell::RefCell::new(Vec::new());
        let input = InputPolicy { prepare: &prepare, notices: &notices };
        let run = |scratch: &mut Session, out: &str| f(scratch, out).map_err(FileFailure::from);
        let options = ProcessOptions { format: Some(&ext), written: Some(&written), ctx: None, input: Some(&input) };
        match process_one(s.prefs().raw_defaults.resolution, &path, &out, save, options, &run) {
            Ok(()) => {
                written.record(&out, &path);
                files.push(out);
            }
            Err(error) => errors.push(error.value("file", &path)),
        }
    }
    json!({"files": files, "errors": errors})
}

struct InputPolicy<'a> {
    prepare: PrepareInput<'a>,
    notices: &'a std::cell::RefCell<Vec<Value>>,
}

struct ProcessOptions<'a> {
    format: Option<&'a str>,
    written: Option<&'a OutputClaims>,
    ctx: Option<&'a crate::jobs::JobCtx>,
    input: Option<&'a InputPolicy<'a>>,
}

/// Open and process one scratch input, collecting opening diagnostics even on failure.
/// All destination writes retain the worker's cancellation checkpoint.
fn process_one(
    raw_ppi: f64,
    path: &str,
    out: &str,
    save: SaveOpts,
    options: ProcessOptions<'_>,
    f: &dyn Fn(&mut Session, &str) -> std::result::Result<bool, FileFailure>,
) -> std::result::Result<(), FileFailure> {
    if let Some(written) = options.written {
        written.check(out).map_err(EngineError::Other)?;
    }
    let mut scratch = Session::new();
    let result = (|| -> std::result::Result<(), FileFailure> {
        if let Some(input) = options.input {
            (input.prepare)(&mut scratch, path)?;
        } else {
            let bytes = read_file(path)?;
            let doc = import_at_raw_ppi(raw_ppi, &file_name(path), &bytes)?;
            scratch.add_document(doc, Some(path.to_string()));
        }
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
                return Err(EngineError::Cancelled.into());
            }
        }
        let d = scratch.active().ok_or(EngineError::NoDocument)?;
        let warnings = if let Some(format) = options.format {
            let ext = if format == "same" { same_format(path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
            if let Some(ctx) = options.ctx { save_batch_doc(&d.doc, out, &ext, save, ctx)? } else { save_doc_as(&d.doc, out, &ext, save)? }
        } else {
            save_doc(&d.doc, out, save)?
        };
        record_export_warnings(&mut scratch, out, warnings);
        Ok(())
    })();
    if let Some(input) = options.input {
        input.notices.borrow_mut().extend(scratch.batch_open_notices.take().unwrap_or_default());
    }
    result
}

/// Encode before the Batch write checkpoint. Esc accepted during an action or encode therefore
/// leaves the current destination untouched; the worker owns the write once the checkpoint passes.
fn save_batch_doc(doc: &Document, out: &str, format: &str, save: SaveOpts, ctx: &crate::jobs::JobCtx) -> Result<Vec<String>> {
    let (bytes, warnings) = encode(doc, format, save)?;
    ctx.batch_before_write()?;
    write_file(out, &bytes)?;
    Ok(warnings)
}

/// Keep encoder diagnostics beside the existing per-file opening notices, including snapshots.
fn record_export_warnings(scratch: &mut Session, out: &str, warnings: Vec<String>) {
    if !warnings.is_empty() {
        scratch.batch_open_notices.get_or_insert_with(Vec::new).push(json!({"file": out, "warnings": warnings}));
    }
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

/// Exercise the native runner without recorded opening or collision-policy overrides.
#[cfg(test)]
pub(super) fn process_batch_outputs(
    raw_ppi: f64,
    outputs: Vec<(String, String)>,
    format: &str,
    save: SaveOpts,
    policy: &BatchPolicy,
    ctx: &crate::jobs::JobCtx,
    f: &dyn Fn(&mut Session, &str) -> Result<bool>,
) -> Result<Value> {
    process_batch_outputs_with(BatchRun { raw_ppi, format, save, policy, input: None, check_duplicates: false }, outputs, ctx, &|scratch, out| {
        f(scratch, out).map_err(FileFailure::from)
    })
}

struct BatchRun<'a> {
    raw_ppi: f64,
    format: &'a str,
    save: SaveOpts,
    policy: &'a BatchPolicy,
    input: Option<&'a InputPolicy<'a>>,
    check_duplicates: bool,
}

/// Share background progress, interruption and write ordering across native and recorded inputs.
/// Recorded opening and destination policy vary without replacing those lifecycle checks.
fn process_batch_outputs_with(
    config: BatchRun<'_>,
    outputs: Vec<(String, String)>,
    ctx: &crate::jobs::JobCtx,
    f: &dyn Fn(&mut Session, &str) -> std::result::Result<bool, FileFailure>,
) -> Result<Value> {
    let BatchRun { raw_ppi, format, save, policy, input, check_duplicates } = config;
    let mut written = OutputClaims::default();
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
        let result = process_one(
            raw_ppi,
            path,
            out,
            save,
            ProcessOptions { format: Some(format), written: check_duplicates.then_some(&written), ctx: Some(ctx), input },
            f,
        );
        if result.is_ok() {
            written.record(out, path);
        }
        // An input that failed on its own (not because it was stopped) stays a failure with its
        // reason, even when a cancel or Esc lands while it runs.
        let failure = result.as_ref().err().filter(|error| !matches!(&error.error, EngineError::Cancelled)).map(|error| error.value("file", path));
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
                errors.push(error.value("file", path));
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
/// case, because macOS and Windows file systems do, and through symlinked folders, so two
/// spellings of one destination are one claim.
#[derive(Default)]
pub struct OutputClaims(pub(super) std::collections::HashMap<String, String>);

impl OutputClaims {
    fn key(out: &str) -> String {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(identity) = super::paths::destination_identity(std::path::Path::new(out)) {
            return identity.to_string_lossy().to_lowercase();
        }
        out.to_lowercase()
    }
    /// `Err` (naming the earlier input) when `out` was already written in this run.
    pub fn check(&self, out: &str) -> std::result::Result<(), String> {
        match self.0.get(&Self::key(out)) {
            Some(first) => Err(format!("not written: {out} already holds the result of {first} from this run (same output name)")),
            None => Ok(()),
        }
    }
    /// Remember that `input`'s result was written to `out`.
    pub fn record(&mut self, out: &str, input: &str) {
        self.0.insert(Self::key(out), input.to_string());
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

/// Refuses a batch or droplet nested past [`MAX_BATCH_DEPTH`](crate::actions_cmds::MAX_BATCH_DEPTH),
/// before it reads or decodes anything: each level keeps its parent's scratch document open.
pub(crate) fn check_batch_depth(s: &Session, cmd: &str) -> Result<()> {
    if s.actions.batch_depth >= crate::actions_cmds::MAX_BATCH_DEPTH {
        return Err(EngineError::BatchLimit(format!(
            "invalid parameters for `{cmd}`: batches and droplets are nested more than {} deep",
            crate::actions_cmds::MAX_BATCH_DEPTH
        )));
    }
    Ok(())
}

pub(crate) fn batch_action(list: &[crate::actions_cmds::Action], p: &Value) -> Result<usize> {
    const CMD: &str = "file.automate.batch";
    if let Some(set) = p.get("set").filter(|v| !v.is_null()) {
        let set = set.as_str().ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "set must be a string".into() })?;
        let name =
            p.get("action").and_then(Value::as_str).ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: "a set needs a named action".into() })?;
        return list
            .iter()
            .position(|a| a.name == name && a.set.as_deref() == Some(set))
            .ok_or_else(|| EngineError::BadParams { cmd: CMD.into(), msg: format!("no action named `{name}` in set `{set}`") });
    }
    crate::actions_cmds::resolve(list, p, CMD)
}

/// Apply the selected file-error policy while retaining per-file results for direct callers.
/// Resolve explicit or named steps, then run folder work on scratch documents or edit live tabs.
/// Named actions carry their recursion context and both routes normalize recorded parameters.
pub(super) fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.batch";
    check_batch_depth(s, cmd)?;
    let quiet_raw = match p.get("suppressOpenOptions") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "suppressOpenOptions must be a boolean".into() }),
    };
    let quiet_profiles = match p.get("suppressWarnings") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "suppressWarnings must be a boolean".into() }),
    };
    let override_open = match p.get("overrideOpen") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "overrideOpen must be a boolean".into() }),
    };
    // Save and Close destination adapted from #10026 (@edaldridgePersonal, b96be5300cfa).
    let save_and_close = match p.get("destination") {
        None | Some(Value::Null) => false,
        Some(Value::String(value)) if value == "folder" => false,
        Some(Value::String(value)) if value == "saveAndClose" => true,
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "destination must be folder or saveAndClose".into() }),
    };
    let save_close_tiff_layers = if save_and_close {
        match p.get("tiffLayers") {
            None | Some(Value::Null) => true,
            Some(Value::Bool(value)) => *value,
            _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "tiffLayers must be a boolean".into() }),
        }
    } else {
        false
    };
    if save_and_close {
        if p.get("output").is_some_and(|v| !v.is_null()) {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: "Save and Close writes each source; omit output".into() });
        }
        if p.get("format").is_some_and(|v| !v.is_null() && v.as_str() != Some("same")) {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: "Save and Close requires the source format (same)".into() });
        }
    }
    let include_non_images = match p.get("includeNonImages") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "includeNonImages must be a boolean".into() }),
    };
    let policy = BatchPolicy::parse(p, cmd)?;
    let source = match p.get("source") {
        None | Some(Value::Null) => "folder",
        Some(Value::String(value)) if matches!(value.as_str(), "folder" | "open") => value.as_str(),
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "source must be folder or open".into() }),
    };
    if source == "open" && (save_and_close || override_open || quiet_raw || quiet_profiles || include_non_images) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "recorded folder-opening and Save and Close options require a folder source".into() });
    }
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
    let mut called_name = None;
    let mut steps = if let Some(st) = p.get("steps") {
        parse_steps(st)?
    } else if let Some(act) = p.get("action").filter(|v| v.is_array()) {
        parse_steps(act)?
    } else {
        // The caller selects the source; calledActions supplies only its internal namespace.
        let index = batch_action(&s.actions.list, p)?;
        let (action, _) = crate::actions_cmds::playback_plan(s, &json!({"action": index}))?;
        called_name = Some(action.name.clone());
        action.enabled_steps().map(|(_, (id, params))| (id.clone(), params.clone())).collect()
    };
    let action_name = s
        .action_history
        .as_ref()
        .map(|group| group.name.clone())
        .or_else(|| s.actions.playback_stack.first().cloned())
        .or_else(|| called_name.clone())
        .or_else(|| p.get("action").and_then(Value::as_str).map(str::to_string))
        .or_else(|| p.get("action").and_then(|action| action.get("name")).and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "Batch".into());
    // Preserve the current Save snapshots and view-step policy while adding named actions.
    steps.retain(|(id, _)| batch_keeps_step(id) && id != "file.export.quickExport");
    if let Some(value) = p.get("overrideSaveAs").filter(|v| !v.is_null()) {
        let override_save = value.as_bool().ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "overrideSaveAs must be a boolean".into() })?;
        let has_save = steps.iter().any(|(id, _)| batch_save_point(id));
        if save_and_close && override_save {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: "Save and Close does not represent Override Save As".into() });
        }
        if !save_and_close && override_save != has_save {
            return Err(EngineError::BadParams {
                cmd: cmd.into(),
                msg: if override_save {
                    "this recorded Batch requires a Save step in its action".into()
                } else {
                    "Batch cannot follow action Save destinations without Override Save As".into()
                },
            });
        }
    }
    if let Some((id, _)) = steps.iter().find(|(id, _)| !batch_save_point(id) && crate::commands::find(id).is_none()) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("unknown command `{id}` in the action") });
    }
    let output = if save_and_close || source == "open" {
        match p.get("output") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(path)) => path.clone(),
            Some(_) => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "output must be a folder path".into() }),
        }
    } else {
        str_param(p, "output", cmd)?.to_string()
    };
    let inputs = if source == "folder" { collect_inputs(p, cmd, &output, include_non_images)? } else { Vec::new() };
    if save_and_close
        && inputs.iter().any(|input| s.documents().iter().any(|doc| doc.path.as_deref().is_some_and(|path| super::paths::same_file(&input.path, path))))
    {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: "Save and Close cannot overwrite a source already open in this session".into() });
    }
    let check_folder_format = include_non_images && p.get("input").or_else(|| p.get("files")).is_some_and(Value::is_string);
    let format = p.get("format").and_then(Value::as_str).unwrap_or("same").to_string();
    // A batch inside a played action charges its inputs × steps (at least one each: an input is read,
    // decoded and saved even with no steps) to the playback tree it is in and
    // hands the count on, so nested batches cannot multiply past the limit. A top-level named
    // action starts fresh; standalone explicit steps retain their existing behavior. A batch nested
    // in another batch (a droplet a droplet runs) is charged the same way, so the levels share one budget.
    let named = called_name.is_some();
    let nested = s.actions.playing > 0 || s.actions.batch_depth > 0;
    let mut playback_steps = 0;
    if nested || named {
        let spent = if nested { s.actions.playback_steps } else { 0 };
        playback_steps =
            spent.saturating_add((steps.len().max(1) as u64).saturating_mul(if source == "open" { s.documents().len() } else { inputs.len() } as u64));
        if playback_steps > crate::actions_cmds::MAX_PLAYBACK_STEPS {
            return Err(EngineError::BatchLimit(format!("invalid parameters for `{cmd}`: this batch would exceed the action playback step limit")));
        }
        if nested {
            s.actions.playback_steps = playback_steps;
        }
    }
    let mut playback_stack = s.actions.playback_stack.clone();
    playback_stack.extend(called_name);
    let actions = crate::actions_cmds::ActionState {
        list,
        playing: if named { s.actions.playing.saturating_add(1) } else { s.actions.playing },
        playback_stack,
        playback_steps,
        script_depth: s.actions.script_depth,
        script_steps: s.actions.script_steps,
        budget_shared: nested,
        batch_depth: s.actions.batch_depth.saturating_add(1),
        ..Default::default()
    };
    if source == "open" {
        let save = SaveOpts::from_params(s, p, cmd)?;
        // A caller's recording index belongs to its original library, not an embedded namespace.
        // As in ordinary named playback, suspend it while executing and restore the Batch owner.
        let previous_recording = if named {
            crate::actions_cmds::flush_recording(s)?;
            s.actions.recording.take()
        } else {
            None
        };
        // Swap only execution context, retaining the recorder, revision and script counters.
        let previous_playing = std::mem::replace(&mut s.actions.playing, actions.playing);
        let previous_stack = std::mem::replace(&mut s.actions.playback_stack, actions.playback_stack);
        let previous_steps = std::mem::replace(&mut s.actions.playback_steps, actions.playback_steps);
        // The nesting depth and shared budget of the levels above travel with the run too.
        let previous_shared = std::mem::replace(&mut s.actions.budget_shared, actions.budget_shared);
        let previous_depth = std::mem::replace(&mut s.actions.batch_depth, actions.batch_depth);
        let previous_list = p.get("calledActions").is_some_and(|value| !value.is_null()).then(|| std::mem::replace(&mut s.actions.list, actions.list));
        let owns_history = s.action_history.is_none() && s.prefs().actions.create_single_history_state;
        if owns_history {
            crate::actions_cmds::begin_document_history(s, &action_name);
        }
        let result = batch_open_documents(s, &steps, p, &output, &format, &policy, save);
        if owns_history {
            crate::actions_cmds::finish_document_history(s);
        }
        s.actions.playing = previous_playing;
        s.actions.playback_stack = previous_stack;
        s.actions.budget_shared = previous_shared;
        s.actions.batch_depth = previous_depth;
        if let Some(previous) = previous_list {
            s.actions.list = previous;
        }
        if previous_playing == 0 && !previous_shared {
            s.actions.playback_steps = previous_steps;
        }
        if named {
            s.actions.recording = previous_recording;
        }
        return result;
    }
    let check_duplicates = recorded_output_guard(p);
    let names = if save_and_close { None } else { batch_naming::plan(&inputs, &output, &format, p)? };
    let mut save = SaveOpts::from_params(s, p, cmd)?;
    if save_and_close {
        save.tiff_layers = save_close_tiff_layers;
        save.preserve_alpha = true;
    }
    let authorize = s.authorize;
    let raw_ppi = super::open::raw_open_resolution(s);
    let action_prefs = s.prefs().actions.clone();
    let prefs = s.prefs.clone();
    let color_settings = s.color.settings.clone();
    let raw_settings = s.batch_raw_open.as_ref().unwrap_or(&s.prefs().raw_defaults).clone();
    let script_count = s.actions.script_steps;
    let outputs: Vec<(String, String)> = if save_and_close {
        inputs.iter().map(|input| (input.path.clone(), input.path.clone())).collect()
    } else {
        match names {
            Some(names) => inputs.iter().map(|input| input.path.clone()).zip(names).collect(),
            None => inputs
                .iter()
                .map(|input| {
                    let ext = if format == "same" { same_format(&input.path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
                    (input.path.clone(), join(&output, &format!("{}.{ext}", stem(&input.path))))
                })
                .collect(),
        }
    };
    #[cfg(not(target_arch = "wasm32"))]
    validate_flat_outputs(&outputs)?;
    validate_error_log(&policy, outputs.iter().map(|(path, _)| path.clone()), outputs.iter().map(|(_, path)| path.clone()))?;
    let was_playing = nested;
    #[cfg(test)]
    let history_observer = s.batch_history_observer;
    crate::jobs::run(
        s,
        "Batch",
        false,
        move |ctx| {
            let carried = std::cell::Cell::new(playback_steps);
            let script_steps = std::cell::Cell::new(script_count);
            let notices = std::cell::RefCell::new(Vec::new());
            // Whether this input's source opened read-only; Save and Close must not replace it.
            let read_only_source = std::cell::Cell::new(false);
            let prepare = |scratch: &mut Session, path: &str| -> Result<()> {
                read_only_source.set(false);
                #[cfg(not(target_arch = "wasm32"))]
                if check_folder_format && !super::paths::openable_name(path) {
                    return Err(EngineError::Other(format!("unsupported Batch folder input: {path}")));
                }
                #[cfg(target_arch = "wasm32")]
                let _ = check_folder_format;
                if quiet_profiles || quiet_raw {
                    scratch.prefs = prefs.clone();
                    scratch.color.settings = color_settings.clone();
                    if quiet_profiles {
                        scratch.color.settings.ask_on_mismatch = false;
                        scratch.color.settings.ask_on_missing = false;
                    }
                    scratch.authorize = authorize;
                    if quiet_raw {
                        scratch.batch_raw_open = Some(raw_settings.clone());
                    }
                    scratch.batch_open_notices = Some(Vec::new());
                }
                if override_open {
                    scratch.batch_open = Some(super::open::BatchOpen { path: path.to_string(), sources: Vec::new(), raw_ppi, read_only: false });
                } else {
                    let bytes = read_file(path)?;
                    if quiet_profiles || quiet_raw {
                        super::open::open_bytes_as(scratch, &file_name(path), &bytes, None, Some(path.to_string()))?;
                        read_only_source.set(scratch.active().is_some_and(|state| state.source_read_only));
                    } else {
                        let (doc, read_only) = super::open::import_source_at_raw_ppi(raw_ppi, &file_name(path), &bytes)?;
                        read_only_source.set(read_only);
                        scratch.add_document(doc, Some(path.to_string()));
                    }
                }
                Ok(())
            };
            let input = InputPolicy { prepare: &prepare, notices: &notices };
            let run = |scratch: &mut Session, out: &str| -> std::result::Result<bool, FileFailure> {
                let source_id = scratch.active().map(|state| state.doc.id);
                let source_path = scratch
                    .batch_open
                    .as_ref()
                    .map(|binding| binding.path.clone())
                    .or_else(|| scratch.active().and_then(|state| state.path.clone()))
                    .unwrap_or_default();
                let encode_format = if format == "same" { same_format(&source_path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
                // Save and Close writes over the source, so its bytes must be the source's own
                // format: a source PhotoCraft reads but saves as PSD (Cineon, whose extension has
                // a writer elsewhere) would otherwise get PSD bytes under its own name.
                if save_and_close
                    && !extension(out).is_some_and(|ext| {
                        ext == encode_format
                            && (matches!(ext.as_str(), "psd" | "psb" | "pcraft" | "ora")
                                || photocraft_formats::from_extension(&ext)
                                    .is_some_and(|format| format.capabilities().write == photocraft_formats::Availability::Available))
                    })
                {
                    return Err(EngineError::Other(format!("Save and Close needs Save As for this source format: {out}")).into());
                }
                // Check the actual result, including a saved snapshot after Close, before either write path.
                let validate_result = |scratch: &Session, doc: &Document| -> Result<()> {
                    if scratch.batch_open.as_ref().is_some_and(|binding| binding.sources.is_empty()) {
                        return Err(EngineError::Other("Override Open requires an executed Open step for this input".into()));
                    }
                    if !save_and_close {
                        return Ok(());
                    }
                    let from_source = scratch.batch_open.as_ref().map_or(source_id == Some(doc.id), |binding| binding.sources.contains(&doc.id));
                    if !from_source {
                        return Err(EngineError::Other("Save and Close needs the original source document or its recorded Save snapshot".into()));
                    }
                    // As File > Save does, never replace a source opened read-only (an Affinity,
                    // Krita, XCF or PDN file, even one named `.png`) with a flat encoding.
                    if read_only_source.get() || scratch.batch_open.as_ref().is_some_and(|binding| binding.read_only) {
                        return Err(EngineError::Other(format!("Save and Close cannot replace this source; save it to a separate output folder: {out}")));
                    }
                    // All recorded Opens passed their gates before inspecting the original source,
                    // including when this result is a saved snapshot whose document has been closed.
                    preserve_source(out)?;
                    let tiff = extension(out).and_then(|ext| photocraft_formats::from_extension(&ext)) == Some(photocraft_formats::Format::Tiff);
                    if tiff {
                        if !matches!(
                            doc.pixel_format().mode,
                            photocraft_color::ColorMode::Rgb | photocraft_color::ColorMode::Grayscale | photocraft_color::ColorMode::Cmyk
                        ) || doc.mode == photocraft_color::ColorMode::Multichannel
                        {
                            return Err(EngineError::Other("Save and Close cannot preserve this TIFF color mode".into()));
                        }
                        if !save.tiff_layers && super::save::tiff_would_write_layers(doc) {
                            return Err(EngineError::Other("Save and Close would flatten TIFF layer data; keep tiffLayers enabled".into()));
                        }
                    }
                    Ok(())
                };
                scratch.authorize = authorize;
                scratch.actions = actions.clone();
                scratch.actions.playback_steps = carried.get();
                scratch.actions.script_steps = script_steps.get();
                scratch.prefs.edit(|prefs| prefs.actions = action_prefs.clone());
                let owns_history = scratch.action_history.is_none() && scratch.prefs().actions.create_single_history_state;
                if owns_history {
                    crate::actions_cmds::begin_document_history(scratch, &action_name);
                }
                let result = (|| -> std::result::Result<bool, FileFailure> {
                    let mut saved: Option<Arc<Document>> = None;
                    for (id, params) in &steps {
                        ctx.check()?;
                        if ctx.batch_interrupt_requested() {
                            return Err(EngineError::Cancelled.into());
                        }
                        // A recorded Save is a step too: Batch writes the output for it.
                        charge_script_step(scratch)?;
                        if batch_save_point(id) {
                            // The scratch session never executes a save point, so the gate would not see it.
                            if let Some(gate) = scratch.authorize {
                                gate(id, params)?;
                            }
                            let st = scratch.active_mut().ok_or(EngineError::NoDocument)?;
                            saved = Some(st.doc.clone());
                            // Batch performs the action's Save / Save As (to its destination), so the
                            // document is saved there as far as the steps after it see: a Close recorded
                            // without a save policy (`"save":"ask"`) then closes it. A Save a Copy leaves
                            // the document as changed as it was.
                            if id != "file.saveACopy" {
                                st.saved_revision = st.revision;
                            }
                            continue;
                        }
                        let params = crate::actions_cmds::playback_params(scratch, id, params)?;
                        let r = scratch.execute(id, params)?;
                        // A nested batch, droplet or played action that hit the nesting or shared playback
                        // limit fails this input too, instead of saving it unprocessed.
                        if let Some(limit) = crate::actions_cmds::limit_stop(&r) {
                            // The child result stays, so the files it wrote before the stop are reported.
                            return Err(FileFailure { error: EngineError::BatchLimit(limit), result: Some(r) });
                        }
                        ctx.publish_batch_preview(&source_path, scratch.active().map(|state| state.doc.clone()));
                        // A failed step inside a called action makes this file an error, not a saved result.
                        if let Some(error) = crate::actions_cmds::nested_failure(id, &r) {
                            return Err(FileFailure { error: EngineError::Other(error), result: Some(r) });
                        }
                    }
                    // While the saved document is still open the steps after the save reach the result too
                    // (#3058), so it is saved after the action as usual. Closed after its save, or with another
                    // document left active, it is written as the save found it.
                    match saved {
                        Some(doc) if scratch.active().is_none_or(|d| d.doc.id != doc.id) => {
                            validate_result(scratch, &doc)?;
                            let warnings = save_batch_doc(&doc, out, &encode_format, save, ctx)?;
                            record_export_warnings(scratch, out, warnings);
                            Ok(true)
                        }
                        _ => {
                            validate_result(scratch, &scratch.active().ok_or(EngineError::NoDocument)?.doc)?;
                            Ok(false)
                        }
                    }
                })();
                if owns_history {
                    crate::actions_cmds::finish_document_history(scratch);
                }
                #[cfg(test)]
                if let Some(observe) = history_observer {
                    observe(scratch);
                }
                carried.set(scratch.actions.playback_steps);
                script_steps.set(scratch.actions.script_steps);
                result
            };
            let mut result = process_batch_outputs_with(
                BatchRun { raw_ppi, format: &format, save, policy: &policy, input: Some(&input), check_duplicates },
                outputs,
                ctx,
                &run,
            )?;
            let notices = notices.into_inner();
            if !notices.is_empty() {
                result["notices"] = json!(notices);
            }
            Ok((result, carried.get(), script_steps.get()))
        },
        move |s, (result, carried, script_steps)| {
            if was_playing {
                s.actions.playback_steps = carried;
            }
            s.actions.script_steps = script_steps;
            let mut result = result;
            mark_script_limit(s, &mut result);
            if let (Some(notices), Some(imports)) = (&mut s.batch_open_notices, result.get("notices").and_then(Value::as_array)) {
                notices.extend(imports.iter().cloned());
            }
            Ok(result)
        },
    )
}

/// Preserve the recorded/structured collision policy while ordinary native Batch keeps
/// current main's later-file-wins behavior. The validated request uses this in both source modes.
fn recorded_output_guard(p: &Value) -> bool {
    p.get("destination").and_then(Value::as_str) == Some("saveAndClose")
        || p.get("includeNonImages").and_then(Value::as_bool) == Some(true)
        || p.get("namingComponents").is_some_and(|value| !value.is_null())
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
    let mut written = OutputClaims::default();
    let check_duplicates = recorded_output_guard(p);
    for (index, (id, name, path)) in originals.iter().enumerate() {
        // The document a recorded Save / Save As marked as saved, with its earlier saved
        // revision: when Batch then writes nothing, it reads as changed again.
        let mut marked: Option<(u64, u64)> = None;
        let result = (|| -> std::result::Result<(), FileFailure> {
            // A rejected destination cannot edit the next document either.
            if check_duplicates && let Some(paths) = &planned {
                let out = paths.get(index).ok_or_else(|| EngineError::Other("batch output plan is incomplete".into()))?;
                written.check(out).map_err(EngineError::Other)?;
            }
            let position = s.documents().iter().position(|state| state.doc.id.0 == *id).ok_or(EngineError::NoDocument)?;
            s.set_active(position);
            let mut saved: Option<Arc<Document>> = None;
            for (command, params) in steps {
                // A recorded Save is a step too: Batch writes the output for it.
                charge_script_step(s)?;
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
                let params = crate::actions_cmds::playback_params(s, command, params)?;
                let response = s.execute(command, params)?;
                if let Some(error) = crate::actions_cmds::nested_failure(command, &response) {
                    return Err(FileFailure { error: EngineError::Other(error), result: Some(response) });
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
            written.record(out, path);
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
            errors.push(error.value("document", name));
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
    mark_script_limit(s, &mut result);
    Ok(result)
}

/// A Batch run from a script charges each step it executes to the script's total, as nested
/// playback does (`consume_playback_step`); the Batch itself is only one script step.
fn charge_script_step(s: &mut Session) -> Result<()> {
    if s.actions.script_depth == 0 {
        return Ok(());
    }
    let max = crate::automate_cmds::MAX_SCRIPT_STEPS;
    if s.actions.script_steps >= max {
        return Err(EngineError::Other(format!("the script ran more than {max} steps in all (scripts calling scripts)")));
    }
    s.actions.script_steps += 1;
    Ok(())
}

/// A file that failed once the script's step budget is spent failed because of it, and every
/// later script step would fail the same way: the result says so, so a script running this
/// Batch directly stops with that error (`batch_stop`) instead of reporting success.
fn mark_script_limit(s: &Session, result: &mut Value) {
    let max = crate::automate_cmds::MAX_SCRIPT_STEPS;
    let failed = result.get("errors").and_then(Value::as_array).is_some_and(|errors| !errors.is_empty());
    if s.actions.script_depth > 0 && s.actions.script_steps >= max && failed {
        result["scriptLimitError"] = json!(format!("the script ran more than {max} steps in all (scripts calling scripts)"));
    }
}

/// Check source structure once at the common pre-write seam, after Open authorization and ownership.
fn preserve_source(path: &str) -> Result<()> {
    let bytes = read_file(path)?;
    if photocraft_codecs::has_unretained_source_data(&bytes).map_err(|e| EngineError::Other(format!("cannot inspect source data: {e}")))? {
        return Err(EngineError::Other("Save and Close cannot preserve additional source data; save to a separate output folder".into()));
    }
    if photocraft_formats::detect(path, &bytes) == Some(photocraft_formats::Format::OpenExr) {
        preserve_exr(&bytes)?;
    }
    Ok(())
}

/// Only a single flat, unprefixed RGB(A) source can be preserved by the EXR writer.
fn preserve_exr(bytes: &[u8]) -> Result<()> {
    let parts = photocraft_codecs::exr_info(bytes, &Default::default()).map_err(|e| EngineError::Other(format!("cannot inspect EXR source: {e}")))?;
    if parts.len() != 1
        || parts.first().is_none_or(|p| {
            p.data_window != (0, 0, p.width, p.height)
                || p.display_window != p.data_window
                || p.deep
                || p.view.is_some()
                || !(3..=4).contains(&p.channels.len())
                || p.channels.iter().any(|c| !matches!(c.name.as_str(), "R" | "G" | "B" | "A"))
                || ["R", "G", "B"].iter().any(|name| !p.channels.iter().any(|c| c.name == *name))
        })
    {
        return Err(EngineError::Other(
            "Save and Close cannot preserve this EXR's parts or channels, or its data/display windows; save to a separate output folder".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod action_history_tests {
    use super::*;
    use crate::actions_cmds::Action;
    use std::cell::RefCell;

    #[derive(Debug)]
    struct Observed {
        id: photocraft_doc::DocId,
        past: usize,
        label: String,
        name: String,
        undo_name: String,
    }

    thread_local! { static OBSERVED: RefCell<Vec<Observed>> = const { RefCell::new(Vec::new()) }; }

    /// Inspect the real scratch boundary without retaining or changing the scratch session.
    fn observe(s: &Session) {
        assert!(s.action_history.is_none(), "Batch must release the scratch scope on every exit");
        let st = s.active().unwrap();
        let mut history = st.history.clone();
        let (before, _) = history.undo(st.doc.clone()).unwrap();
        let undo_name = before.layer(st.active_layer.unwrap()).unwrap().name.clone();
        let (after, _) = history.redo(before).unwrap();
        assert_eq!(after.as_ref(), st.doc.as_ref(), "redo restores exact document and layer identities");
        let name = st.doc.layer(st.active_layer.unwrap()).unwrap().name.clone();
        OBSERVED.with(|seen| {
            seen.borrow_mut().push(Observed { id: st.doc.id, past: st.history.past_len(), label: st.history.undo_label().unwrap().into(), name, undo_name })
        });
    }

    /// Batch and droplets keep one independent named entry per input, including nested and stopped runs.
    #[test]
    fn batch_and_droplet_document_history_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let image = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgb, vec![255; 48]).unwrap();
        let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let paths: Vec<_> = ["a.png", "b.png"]
            .iter()
            .map(|name| {
                let path = dir.path().join(name);
                std::fs::write(&path, &bytes).unwrap();
                path
            })
            .collect();
        for (case, single, nested, stopped, droplet, named) in [
            ("batch", true, false, false, false, false),
            ("off", false, false, false, false, false),
            ("nested", true, true, false, false, false),
            ("stopped", true, false, true, false, false),
            ("droplet", true, false, false, true, false),
            ("named", true, false, false, false, true),
            ("named-off", false, false, false, false, true),
            ("named-nested", true, true, false, false, true),
            ("named-stopped", true, false, true, false, true),
        ] {
            OBSERVED.with(|seen| seen.borrow_mut().clear());
            let mut s = Session::new();
            s.batch_history_observer = Some(observe);
            s.execute("actions.setHistory", json!({"singleState":single})).unwrap();
            let mut steps = vec![("layer.renameLayer".into(), json!({"name":"Prefix"}))];
            if stopped {
                steps.push(("actions.stopMessage".into(), json!({"message":"Stop here"})));
            }
            steps.push(("layer.renameLayer".into(), json!({"name":"Final"})));
            let action_steps = if nested {
                s.actions.list.push(Action::new("Child", steps));
                vec![("actions.play".to_string(), json!({"action":"Child"}))]
            } else {
                steps
            };
            let output = dir.path().join(case);
            let result = if droplet {
                let path = dir.path().join("named.pcdroplet");
                s.execute("file.automate.createDroplet", json!({"path":path,"name":"Named","steps":action_steps,"output":output,"shim":false})).unwrap();
                s.execute("file.automate.runDroplet", json!({"droplet":path,"input":paths})).unwrap()
            } else if named {
                let index = s.actions.list.len();
                s.actions.list.push(Action::new("Named", action_steps));
                s.execute("file.automate.batch", json!({"action":index,"input":paths,"output":output})).unwrap()
            } else {
                s.execute("file.automate.batch", json!({"action":"Named","steps":action_steps,"input":paths,"output":output})).unwrap()
            };
            assert_eq!(result["errors"].as_array().unwrap().len(), if stopped { 2 } else { 0 }, "{case}: {result}");
            OBSERVED.with(|seen| {
                let seen = seen.borrow();
                assert_eq!(seen.len(), 2, "{case}");
                assert_ne!(seen[0].id, seen[1].id, "inputs have separate editing identities");
                for entry in seen.iter() {
                    assert_eq!(entry.past, if single { 1 } else { 2 }, "{case}: {entry:?}");
                    if single {
                        assert_eq!(entry.label, "Action: Named", "{case}");
                    }
                    assert_eq!(entry.name, if stopped { "Prefix" } else { "Final" }, "{case}");
                    if !single {
                        assert_eq!(entry.undo_name, "Prefix", "OFF undoes only the last step");
                    } else {
                        assert_ne!(entry.undo_name, "Prefix", "ON undoes the whole play");
                    }
                }
            });
        }
    }

    #[test]
    /// Batch over open documents must preserve an enclosing action scope and finish its own document scopes.
    fn opened_document_batch_keeps_top_level_and_parent_history_boundaries() {
        for (single, parent, named) in [false, true]
            .into_iter()
            .flat_map(|single| [false, true].into_iter().flat_map(move |parent| [false, true].into_iter().map(move |named| (single, parent, named))))
        {
            let mut s = Session::new();
            for _ in 0..2 {
                s.execute("file.new", json!({"width":4,"height":4})).unwrap();
            }
            s.execute("actions.setHistory", json!({"singleState":single})).unwrap();
            let before: Vec<_> = s.documents().iter().map(|st| (st.doc.clone(), st.history.past_len())).collect();
            let steps = vec![("layer.renameLayer".into(), json!({"name":"Prefix"})), ("layer.renameLayer".into(), json!({"name":"Final"}))];
            let params = if named {
                s.actions.list.push(Action::new("Opened batch", steps));
                json!({"source":"open","action":0})
            } else {
                json!({"source":"open","action":"Opened batch","steps":steps})
            };
            if parent {
                s.actions.list.push(Action::new(
                    "Parent",
                    vec![
                        ("layer.renameLayer".into(), json!({"name":"Before batch"})),
                        ("file.automate.batch".into(), params),
                        ("layer.renameLayer".into(), json!({"name":"After batch"})),
                    ],
                ));
                let result = s.execute("actions.play", json!({"action":"Parent"})).unwrap();
                assert!(result.get("failed").is_none(), "{result}");
            } else {
                let result = s.execute("file.automate.batch", params).unwrap();
                assert!(result["errors"].as_array().unwrap().is_empty(), "{result}");
            }
            assert!(s.action_history.is_none());
            for (index, (original, past)) in before.iter().enumerate() {
                s.set_active(index);
                let st = s.active().unwrap();
                let after = st.doc.clone();
                assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, if parent && index == 1 { "After batch" } else { "Final" });
                let steps = if parent && index == 1 { 4 } else { 2 };
                assert_eq!(st.history.past_len(), past + if single { 1 } else { steps });
                if single {
                    assert_eq!(st.history.undo_label(), Some(if parent { "Action: Parent" } else { "Action: Opened batch" }));
                }
                assert!(s.undo());
                if single {
                    assert_eq!(s.active().unwrap().doc.as_ref(), original.as_ref());
                } else {
                    assert_ne!(s.active().unwrap().doc.as_ref(), original.as_ref());
                }
                assert!(s.redo());
                assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
            }
        }
    }

    #[test]
    /// A Batch failure restores the library and leaves the caller's active history scope owned by the caller.
    fn opened_document_batch_error_restores_library_without_finishing_parent() {
        for parent in [false, true] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":4,"height":4})).unwrap();
            s.execute("actions.setHistory", json!({"singleState":true})).unwrap();
            s.actions.list.push(Action::new("Original", vec![]));
            let library = serde_json::to_value(&s.actions.list).unwrap();
            let before = s.active().unwrap().doc.clone();
            if parent {
                crate::actions_cmds::begin_document_history(&mut s, "Parent");
            }
            let result = s
                .execute(
                    "file.automate.batch",
                    json!({
                        "source":"open","action":"Broken","calledActions":[Action::new("Foreign", vec![])],
                        "steps":[["layer.renameLayer",{"name":"Prefix"}],["layer.renameLayer",{"name":false}]]
                    }),
                )
                .unwrap();
            assert_eq!(result["errors"].as_array().unwrap().len(), 1, "{result}");
            assert_eq!(serde_json::to_value(&s.actions.list).unwrap(), library);
            if parent {
                assert_eq!(s.action_history.as_ref().unwrap().name, "Parent");
                s.execute("layer.renameLayer", json!({"name":"Parent tail"})).unwrap();
                crate::actions_cmds::finish_document_history(&mut s);
            } else {
                assert!(s.action_history.is_none());
            }
            let st = s.active().unwrap();
            let after = st.doc.clone();
            assert_eq!(st.history.undo_label(), Some(if parent { "Action: Parent" } else { "Action: Broken" }));
            assert!(s.undo());
            assert_eq!(s.active().unwrap().doc.as_ref(), before.as_ref());
            assert!(s.redo());
            assert_eq!(s.active().unwrap().doc.as_ref(), after.as_ref());
        }
    }

    thread_local! { static CANCEL_CONTEXT: RefCell<Option<crate::jobs::JobCtx>> = const { RefCell::new(None) }; }

    fn cancel_after_prefix(id: &str, params: &Value) -> Result<()> {
        if id == "layer.renameLayer" && params["name"] == "Cancel here" {
            CANCEL_CONTEXT.with(|ctx| ctx.borrow().as_ref().unwrap().cancel());
            return Err(EngineError::Cancelled);
        }
        Ok(())
    }

    #[test]
    /// Cancellation after a completed edit retains an undoable prefix without leaving a Batch-owned scope open.
    fn cancelled_folder_batch_finalizes_its_completed_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let image = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgb, vec![255; 48]).unwrap();
        let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let input = dir.path().join("input.png");
        std::fs::write(&input, bytes).unwrap();
        let mut s = Session::new();
        s.execute("actions.setHistory", json!({"singleState":true})).unwrap();
        let ctx = crate::jobs::JobCtx::new();
        s.set_inline_job_ctx(Some(ctx.clone()));
        CANCEL_CONTEXT.with(|slot| *slot.borrow_mut() = Some(ctx.clone()));
        OBSERVED.with(|seen| seen.borrow_mut().clear());
        s.authorize = Some(cancel_after_prefix);
        s.batch_history_observer = Some(observe);
        let result = s
            .execute(
                "file.automate.batch",
                json!({"action":"Cancelled batch","input":[input],"output":dir.path().join("out"),
                    "steps":[["layer.renameLayer",{"name":"Prefix"}],["layer.renameLayer",{"name":"Cancel here"}]]
                }),
            )
            .unwrap();
        CANCEL_CONTEXT.with(|slot| *slot.borrow_mut() = None);
        assert!(ctx.cancelled());
        assert_eq!(result["cancelled"], true, "{result}");
        assert!(result["files"].as_array().unwrap().is_empty(), "{result}");
        OBSERVED.with(|seen| {
            let seen = seen.borrow();
            assert_eq!(seen.len(), 1);
            assert_eq!(seen[0].past, 1);
            assert_eq!(seen[0].label, "Action: Cancelled batch");
            assert_eq!(seen[0].name, "Prefix");
            assert_ne!(seen[0].undo_name, "Prefix");
        });
    }
}
