//! Headless JSON-lines server: the control protocol's envelope
//! (`{"id","method","params"}` → `{"id","ok","result"|"error"}`) over a
//! [`Headless`] session, so scripts and agents can keep one editing session
//! open without MCP or the GUI. `photocraft-cli serve` runs it on stdio or a
//! loopback TCP port.
//!
//! Methods (camelCase params):
//! - `engine.execute {command, params?, wait?}` / `engine.commands {filter?}`
//! - `jobs.list` / `jobs.cancel {job?}`: background jobs (#210)
//! - `session.list`, `doc.open {path}`, `doc.new {…file.new params}`,
//!   `doc.place {path, scale?, fit?, center?, index?}` (#2810: the read root is the scope,
//!   the counterpart of the refused ambient `file.placeEmbedded`),
//!   `doc.replaceSmartObject {path, layer?, index?}` (#8061: the same scope for
//!   `layer.smartObjects.replaceContents`),
//!   `doc.exportLayers {dir, format?, prefix?, visibleOnly?, quality?, index?}`,
//!   `doc.exportDataSets {dir, format?, dataSets?, naming?, index?}`,
//!   `doc.importDataSets {path, delimiter?, index?}` (#3216: the same treatment for the export
//!   and CSV half of #2810), `doc.save {path?, format?, quality?, index?}`, `doc.inspect {index?}`,
//!   `doc.render {index?, maxSide?, path?}` (writes a PNG to `path`, else
//!   returns it base64-encoded), `doc.select {index}`, `doc.close {index?}`
//! - `batch {steps: [{command, params?, wait?} | {method, params?}], stopOnError?}`
//! - `methods`: this list.

use std::io::{BufRead, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

use crate::budgets::{BatchReplyBudget, write_reply};
use crate::security::{
    AUTH_DEADLINE, AuthDeadlineStream, ConnectionLimiter, LineRead, MAX_BATCH_STEPS, MAX_CONNECTIONS, MAX_REQUEST_BYTES, authentication_reply,
    configure_stream, discard_rest_of_line, read_bounded_line, repeat_auth_reply,
};
use crate::{AutomationError, Headless};

/// Method names served by [`Headless::handle`].
pub const METHODS: &[&str] = &[
    "engine.execute",
    "jobs.list",
    "jobs.cancel",
    "engine.commands",
    "session.list",
    "doc.open",
    "doc.place",
    "doc.replaceSmartObject",
    "doc.new",
    "doc.save",
    "doc.exportLayers",
    "doc.exportDataSets",
    "doc.importDataSets",
    "doc.inspect",
    "doc.render",
    "doc.select",
    "doc.close",
    "batch",
    "methods",
];

fn bad(msg: impl Into<String>) -> AutomationError {
    AutomationError::BadRequest(msg.into())
}

/// The optional `index` param: missing or null means the active document; anything else must be a
/// non-negative integer (never silently fall back to the active document, which `doc.close` would close).
fn index_of(p: &Value) -> Result<Option<usize>, AutomationError> {
    match p.get("index") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).map(Some).ok_or_else(|| bad("`index` must be a non-negative integer")),
    }
}

fn str_of<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

impl Headless {
    /// Dispatch one request. Unknown methods and bad params are errors, never panics.
    pub fn handle(&mut self, method: &str, params: Value) -> Result<Value, AutomationError> {
        self.sync_jobs();
        let p = if params.is_null() { json!({}) } else { params };
        match method {
            "engine.execute" => {
                let id = str_of(&p, "command").ok_or_else(|| bad("engine.execute needs `command`"))?;
                if METHODS.contains(&id) && photocraft_engine::commands::find(id).is_none() {
                    return Err(bad(photocraft_engine::command_hints::protocol_method_as_command(id)));
                }
                let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
                self.command_start(id, p.get("params").cloned().unwrap_or(Value::Null), wait)
            }
            // Background jobs (#210): list (applying any that finished) and cancel.
            "jobs.list" => self.command_start("jobs.list", json!({}), true),
            "jobs.cancel" => self.command_start("jobs.cancel", p, true),
            "engine.commands" => {
                let all = self.command_list();
                Ok(match str_of(&p, "filter").map(str::to_lowercase) {
                    None => all,
                    Some(n) => {
                        Value::Array(all.as_array().into_iter().flatten().filter(|c| photocraft_engine::commands::list_entry_matches(c, &n)).cloned().collect())
                    }
                })
            }
            "session.list" => Ok(self.session_list()),
            "doc.open" => {
                let path = str_of(&p, "path").ok_or_else(|| bad("doc.open needs `path`"))?;
                self.open(&PathBuf::from(path))
            }
            "doc.place" => {
                let path = str_of(&p, "path").ok_or_else(|| bad("doc.place needs `path`"))?;
                let mut opts = p.as_object().cloned().unwrap_or_default();
                opts.remove("path");
                opts.remove("index");
                self.place(index_of(&p)?, path, &Value::Object(opts))
            }
            "doc.replaceSmartObject" => {
                let path = str_of(&p, "path").ok_or_else(|| bad("doc.replaceSmartObject needs `path`"))?;
                let mut opts = p.as_object().cloned().unwrap_or_default();
                opts.remove("path");
                opts.remove("index");
                self.replace_smart_object(index_of(&p)?, path, &Value::Object(opts))
            }
            "doc.new" => self.command_run("file.new", p),
            "doc.save" => {
                let tga_bits = photocraft_engine::file_cmds::tga_bits_param(&p, "doc.save").map_err(|e| bad(e.to_string()))?;
                let mut opts =
                    photocraft_io::ExportOptions { tiff_layers: p.get("tiffLayers").and_then(Value::as_bool).unwrap_or(false), tga_bits, ..Default::default() };
                opts.encode.dpx_bits = photocraft_engine::file_cmds::dpx_bits_param(&p, "doc.save").map_err(|e| bad(e.to_string()))?;
                opts.encode.dds = photocraft_engine::file_cmds::dds_param(&p, "doc.save").map_err(|e| bad(e.to_string()))?.unwrap_or_default();
                match p.get("quality") {
                    None | Some(Value::Null) => {}
                    Some(v) => {
                        // Like the CLI's `--quality`: never ignore or clamp a bad value.
                        let q = v
                            .as_u64()
                            .filter(|q| (1..=100).contains(q))
                            .and_then(|q| u8::try_from(q).ok())
                            .ok_or_else(|| bad(format!("`quality` must be a whole number from 1 to 100, got {v}")))?;
                        opts.encode.jpeg_quality = q;
                        opts.encode.webp_quality = q;
                        opts.encode.webp_lossless = false;
                    }
                }
                let path = str_of(&p, "path").map(PathBuf::from);
                self.save(index_of(&p)?, path.as_deref(), str_of(&p, "format"), &opts)
            }
            "doc.exportLayers" | "doc.exportDataSets" => {
                let dir = str_of(&p, "dir").ok_or_else(|| bad(format!("{method} needs `dir`")))?;
                let mut opts = p.as_object().cloned().unwrap_or_default();
                opts.remove("dir");
                opts.remove("index");
                let opts = Value::Object(opts);
                if method == "doc.exportLayers" { self.export_layers(index_of(&p)?, dir, &opts) } else { self.export_data_sets(index_of(&p)?, dir, &opts) }
            }
            "doc.importDataSets" => {
                let path = str_of(&p, "path").ok_or_else(|| bad("doc.importDataSets needs `path`"))?;
                let mut opts = p.as_object().cloned().unwrap_or_default();
                opts.remove("path");
                opts.remove("index");
                self.import_data_sets(index_of(&p)?, path, &Value::Object(opts))
            }
            "doc.inspect" => self.inspect(index_of(&p)?),
            "doc.render" => {
                let max = match p.get("maxSide") {
                    None => 1024,
                    Some(value) => {
                        value.as_u64().and_then(|value| u32::try_from(value).ok()).ok_or_else(|| bad("maxSide must be an unsigned 32-bit integer"))?
                    }
                };
                let png = self.render_png(index_of(&p)?, max)?;
                match str_of(&p, "path") {
                    Some(path) => {
                        self.write_render(&PathBuf::from(path), &png)?;
                        Ok(json!({"path": path, "bytes": png.len()}))
                    }
                    None => Ok(json!({
                        "mime": "image/png",
                        "base64": base64::engine::general_purpose::STANDARD.encode(&png),
                    })),
                }
            }
            "doc.select" => {
                let i = index_of(&p)?.ok_or_else(|| bad("doc.select needs `index`"))?;
                self.select(i)
            }
            "doc.close" => self.close(index_of(&p)?),
            "batch" => self.batch(&p),
            "methods" => Ok(json!(METHODS)),
            other => Err(bad(format!("{} (try `methods`)", photocraft_engine::command_hints::unknown_method(other, METHODS)))),
        }
    }

    /// Run `steps` in order. Each step is `{command, params?, wait?}` (an engine command) or
    /// `{method, params?}` (any [`METHODS`] entry). Stops at the first error unless
    /// `stopOnError` is false; the reply lists every step's result.
    pub fn batch(&mut self, p: &Value) -> Result<Value, AutomationError> {
        self.batch_with_budget(p, BatchReplyBudget::default())
    }

    /// [`Headless::batch`] with a caller's reply budget (MCP charges escaped sizes, see
    /// [`BatchReplyBudget::escaped`]). Steps stop once the budget runs out.
    pub fn batch_with_budget(&mut self, p: &Value, mut reply_budget: BatchReplyBudget) -> Result<Value, AutomationError> {
        let steps = p.get("steps").and_then(Value::as_array).ok_or_else(|| bad("batch needs `steps`"))?;
        if steps.len() > MAX_BATCH_STEPS {
            return Err(bad(format!("batch contains {} steps; maximum is {MAX_BATCH_STEPS}", steps.len())));
        }
        let stop = p.get("stopOnError").and_then(Value::as_bool).unwrap_or(true);
        let mut results = Vec::with_capacity(steps.len());
        let mut failed = 0usize;
        for (i, s) in steps.iter().enumerate() {
            let params = s.get("params").cloned().unwrap_or(Value::Null);
            let r = if let Some(c) = str_of(s, "command") {
                // Like `engine.execute`: `wait: false` starts a long command as a background job.
                let wait = s.get("wait").and_then(Value::as_bool).unwrap_or(true);
                self.command_start(c, params, wait)
            } else if let Some(m) = str_of(s, "method") {
                if m == "batch" { Err(bad("nested batch")) } else { self.handle(m, params) }
            } else {
                Err(bad(format!("step {i} needs `command` or `method`")))
            };
            let was_error = r.is_err();
            let result = match r {
                Ok(v) => json!({"ok": true, "result": v}),
                Err(e) => json!({"ok": false, "error": e.to_string()}),
            };
            if let Err(error) = reply_budget.charge(&result) {
                failed += 1;
                results.push(json!({"ok": false, "error": format!("batch response budget exceeded; current step may have completed: {error}")}));
                break;
            }
            results.push(result);
            if was_error {
                failed += 1;
                if stop {
                    break;
                }
            }
        }
        Ok(json!({"completed": results.len() - failed, "failed": failed, "results": results}))
    }
}

/// Answer one request line. Malformed JSON gets an error reply with `id: null`.
pub fn respond(h: &Mutex<Headless>, line: &str) -> Value {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return json!({"id": null, "ok": false, "error": format!("bad JSON: {e}")}),
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let Some(method) = req.get("method").and_then(Value::as_str) else {
        return json!({"id": id, "ok": false, "error": "missing `method`"});
    };
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    // Last-resort guard (AGENTS.md, Never crash): a command that panics answers this request
    // with an error instead of taking down the connection (or the whole stdio server). Edits
    // run on a copy of the document (`Session::edit`), so the session stays consistent and a
    // poisoned lock is safe to keep using, the same as the MCP server does.
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.lock().unwrap_or_else(PoisonError::into_inner).handle(method, params)))
        .unwrap_or_else(|_| Err(AutomationError::Other(format!("internal error: `{method}` panicked"))));
    match r {
        Ok(v) => json!({"id": id, "ok": true, "result": v}),
        Err(e) => json!({"id": id, "ok": false, "error": e.to_string()}),
    }
}

/// Serve JSON lines from `r` to `w` until EOF. Blank lines are ignored. An over-long or non-UTF-8
/// line gets an error reply and is skipped; the session (and its open documents) keeps serving.
pub fn serve_lines(h: &Mutex<Headless>, mut r: impl BufRead, mut w: impl Write) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        let (reply, unread_rest) = match read_bounded_line(&mut r, &mut line) {
            Ok(LineRead::Eof) => return Ok(()),
            Ok(LineRead::TooLong) => (json!({"id": null, "ok": false, "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes")}), !line.ends_with('\n')),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => (json!({"id": null, "ok": false, "error": "request is not valid UTF-8"}), false),
            Err(e) => return Err(e),
            Ok(LineRead::Line) if line.trim().is_empty() => continue,
            Ok(LineRead::Line) => (respond(h, &line), false),
        };
        write_reply(&mut w, &reply)?;
        w.flush()?;
        if unread_rest {
            discard_rest_of_line(&mut r)?;
        }
    }
}

/// Serve one connection. It must complete `auth` within `auth_deadline` of connecting, however
/// much it sends meanwhile (#4965); after that only the idle timeout applies.
fn serve_tcp_connection(h: &Mutex<Headless>, stream: std::net::TcpStream, token: &str, auth_deadline: std::time::Duration) -> std::io::Result<()> {
    configure_stream(&stream)?;
    let read = stream.try_clone()?;
    let mut reader = std::io::BufReader::new(AuthDeadlineStream::new(read, auth_deadline));
    let mut out = stream;
    let mut line = String::new();
    let mut authenticated = false;
    loop {
        match read_bounded_line(&mut reader, &mut line)? {
            LineRead::Eof => return Ok(()),
            LineRead::TooLong => {
                let reply = json!({
                    "id": null,
                    "ok": false,
                    "error": format!("request exceeds {MAX_REQUEST_BYTES} bytes"),
                });
                write_reply(&mut out, &reply)?;
                out.flush()?;
                return Ok(());
            }
            LineRead::Line if line.trim().is_empty() => continue,
            LineRead::Line => {}
        }
        let reply = if authenticated {
            repeat_auth_reply(&line).unwrap_or_else(|| respond(h, &line))
        } else {
            let (reply, ok) = authentication_reply(&line, token);
            authenticated = ok;
            if ok {
                reader.get_mut().authenticated()?;
            }
            reply
        };
        write_reply(&mut out, &reply)?;
        out.flush()?;
        if !authenticated {
            return Ok(());
        }
    }
}

/// Headless TCP daemons with no clients should eventually release their listening port (#3682).
const TCP_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// Serve authenticated JSON lines on a loopback TCP address with one shared session.
/// Refuses non-loopback addresses, caps connections and request bytes, and closes the listener
/// after an idle period with no connections. Active clients are never evicted by this timer.
pub fn serve_tcp(addr: &str, h: Arc<Mutex<Headless>>, token: String, ready: impl FnOnce(std::net::SocketAddr)) -> Result<(), AutomationError> {
    serve_tcp_until_idle(addr, h, token, ready, TCP_IDLE_TIMEOUT)
}

fn serve_tcp_until_idle(
    addr: &str,
    h: Arc<Mutex<Headless>>,
    token: String,
    ready: impl FnOnce(std::net::SocketAddr),
    idle: Duration,
) -> Result<(), AutomationError> {
    let listener = TcpListener::bind(addr).map_err(|e| AutomationError::Io(format!("bind {addr}: {e}")))?;
    let local = listener.local_addr().map_err(|e| AutomationError::Io(e.to_string()))?;
    if !local.ip().is_loopback() {
        return Err(bad(format!("{addr} is not a loopback address")));
    }
    listener.set_nonblocking(true).map_err(|e| AutomationError::Io(e.to_string()))?;
    ready(local);
    let limiter = ConnectionLimiter::new(MAX_CONNECTIONS);
    let token = Arc::new(token);
    let active = Arc::new(AtomicUsize::new(0));
    let mut last_active = Instant::now();
    loop {
        match listener.accept() {
            Ok((mut stream, _)) => {
                last_active = Instant::now();
                let Some(permit) = limiter.try_acquire() else {
                    let _ = configure_stream(&stream);
                    let _ = writeln!(stream, "{}", json!({"id": null, "ok": false, "error": "connection limit reached"}));
                    continue;
                };
                // The listener is nonblocking; individual requests still use blocking socket I/O.
                let _ = stream.set_nonblocking(false);
                let h = h.clone();
                let token = Arc::clone(&token);
                let active = Arc::clone(&active);
                active.fetch_add(1, Ordering::AcqRel);
                std::thread::spawn(move || {
                    let _permit = permit;
                    let _ = serve_tcp_connection(&h, stream, &token, AUTH_DEADLINE);
                    active.fetch_sub(1, Ordering::AcqRel);
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if active.load(Ordering::Acquire) != 0 {
                    last_active = Instant::now();
                } else if last_active.elapsed() >= idle {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::Interrupted | std::io::ErrorKind::ConnectionAborted) => {}
            Err(e) => return Err(AutomationError::Io(e.to_string())),
        }
    }
}

#[cfg(test)]
mod tests;
