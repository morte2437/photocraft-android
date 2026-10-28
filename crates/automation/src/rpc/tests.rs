use super::*;

fn session() -> Mutex<Headless> {
    Mutex::new(Headless::trusted_local())
}

#[test]
fn document_activation_reply_survives_engine_rpc() {
    let h = session();
    let mut g = h.lock().unwrap();
    for _ in 0..2 {
        g.handle("engine.execute", json!({"command": "file.new", "params": {"width": 4, "height": 4}})).unwrap();
    }
    let reply = g.handle("engine.execute", json!({"command": "document.activate", "params": {"offset": -1}})).unwrap();
    assert_eq!(reply, json!({"document": 0}));
    let reply = g.handle("engine.execute", json!({"command": "document.activate", "params": {"document": 1}})).unwrap();
    assert_eq!(reply, json!({"document": 1}));
}

#[test]
fn execute_reports_invalid_filter_parameters_without_changing_history() {
    let h = session();
    assert_eq!(respond(&h, r#"{"method":"doc.new","params":{"width":8,"height":8}}"#)["ok"], true);
    let before = h.lock().unwrap().inspect(None).unwrap();
    for (params, key) in
        [(json!({"radius":"big"}), "radius"), (json!({"radus":3}), "radus"), (json!({"radius":5000}), "radius"), (json!({"radius":-4}), "radius")]
    {
        for wait in [true, false] {
            let request = json!({"method":"engine.execute","params":{"command":"filter.blur.gaussianBlur","params":params,"wait":wait}});
            let reply = respond(&h, &request.to_string());
            assert_eq!(reply["ok"], false);
            let error = reply["error"].as_str().unwrap();
            assert!(error.contains("filter.blur.gaussianBlur") && error.contains(key), "{error}");
            assert_eq!(h.lock().unwrap().inspect(None).unwrap(), before);
        }
    }
}

/// Pins the reply shape documented in docs/control-protocol.md › Command list replies
/// (#3684): a bare JSON array (not `{"commands": [...]}`), one object per command with
/// exactly these keys (plus an optional `keywords` string), from `engine.commands` (filtered or not) and `command.list`.
#[test]
fn command_list_replies_are_documented_array_of_objects() {
    let h = session();
    let mut g = h.lock().unwrap();
    let all = g.handle("engine.commands", json!({})).unwrap();
    let filtered = g.handle("engine.commands", json!({"filter": "BLUR"})).unwrap();
    let engine = g.handle("engine.execute", json!({"command": "command.list"})).unwrap();
    for (name, reply) in [("engine.commands", &all), ("engine.commands {filter}", &filtered), ("command.list", &engine)] {
        let items = reply.as_array().unwrap_or_else(|| panic!("{name} must reply with a bare array, got {reply}"));
        assert!(!items.is_empty(), "{name}");
        for c in items {
            // `keywords` is optional: only commands with other search names carry it (#5060).
            let mut keys: Vec<&str> = c.as_object().unwrap().keys().map(String::as_str).filter(|k| *k != "keywords").collect();
            keys.sort_unstable();
            assert!(c.get("keywords").is_none_or(Value::is_string), "{name}: {c}");
            assert_eq!(keys, ["disabledReason", "enabled", "id", "label", "menu", "needsDocument", "params", "shortcut"], "{name}: {c}");
            assert!(c["id"].is_string() && c["label"].is_string() && c["params"].is_string() && c["enabled"].is_boolean(), "{c}");
            assert!(c["needsDocument"].is_boolean(), "{c}");
            assert_eq!(c["disabledReason"].is_null(), c["enabled"] == true, "disabledReason is null exactly when enabled: {c}");
            assert!(c["menu"].as_array().unwrap().iter().all(Value::is_string), "{c}");
            assert!(c["shortcut"].is_string() || c["shortcut"].is_null(), "{c}");
        }
    }
    let n_all = all.as_array().unwrap().len();
    assert_eq!(engine.as_array().unwrap().len(), n_all);
    let filtered = filtered.as_array().unwrap();
    assert!(filtered.len() < n_all);
    assert!(filtered.iter().all(|c| format!("{} {}", c["id"], c["label"]).to_lowercase().contains("blur")));
    let new = all.as_array().unwrap().iter().find(|c| c["id"] == "file.new").unwrap();
    assert_eq!((&new["menu"], &new["shortcut"], &new["enabled"]), (&json!(["File"]), &json!("Cmd+N"), &json!(true)), "{new}");
    assert_eq!((&new["disabledReason"], &new["needsDocument"]), (&Value::Null, &json!(false)), "{new}");
}

#[test]
fn lines_round_trip_and_errors() {
    let h = session();
    let input = concat!(
        r#"{"id":1,"method":"doc.new","params":{"width":64,"height":32,"background":"white"}}"#,
        "\n",
        "\n",
        r#"{"id":2,"method":"engine.execute","params":{"command":"layer.new.layer","params":{"name":"Ink"}}}"#,
        "\n",
        r#"{"id":3,"method":"doc.inspect"}"#,
        "\n",
        r#"{"id":4,"method":"nope"}"#,
        "\n",
        "not json\n",
    );
    let mut out = Vec::new();
    serve_lines(&h, input.as_bytes(), &mut out).unwrap();
    let replies: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.len(), 5);
    assert!(replies[0]["ok"].as_bool().unwrap());
    assert_eq!(replies[1]["id"], 2);
    assert!(replies[2]["result"].to_string().contains("Ink"));
    assert!(!replies[3]["ok"].as_bool().unwrap());
    assert!(replies[3]["error"].as_str().unwrap().contains("unknown method"));
    assert_eq!(replies[4]["id"], Value::Null);
}

/// Regression: after a panic while the session lock was held, every later request
/// failed with "session lock poisoned"; the server now keeps serving.
#[test]
fn poisoned_session_keeps_serving() {
    let h = session();
    let _ = std::thread::scope(|sc| {
        sc.spawn(|| {
            let _g = h.lock().unwrap();
            panic!("poison the session lock");
        })
        .join()
    });
    assert!(h.is_poisoned());
    let r = respond(&h, r#"{"id":1,"method":"doc.new","params":{"width":8,"height":8}}"#);
    assert_eq!(r["ok"], true, "{r}");
}

#[test]
fn batch_stops_on_error_unless_told_not_to() {
    let h = session();
    let mut g = h.lock().unwrap();
    g.handle("doc.new", json!({"width": 16, "height": 16})).unwrap();
    let steps = json!([
        {"command": "layer.new.layer", "params": {"name": "A"}},
        {"command": "no.such.command"},
        {"command": "layer.new.layer", "params": {"name": "B"}},
        {"method": "doc.inspect"}
    ]);
    let r = g.handle("batch", json!({"steps": steps})).unwrap();
    assert_eq!(r["completed"], 1);
    assert_eq!(r["failed"], 1);
    assert_eq!(r["results"].as_array().unwrap().len(), 2);
    let r = g.handle("batch", json!({"steps": steps, "stopOnError": false})).unwrap();
    assert_eq!(r["completed"], 3);
    let insp = &r["results"][3]["result"];
    assert!(insp.to_string().contains("\"B\""));
}

#[test]
fn batch_rejects_too_many_steps() {
    let h = session();
    let mut g = h.lock().unwrap();
    let steps = vec![json!({"command": "command.list"}); MAX_BATCH_STEPS + 1];
    let error = g.handle("batch", json!({"steps": steps})).unwrap_err();
    assert!(error.to_string().contains("maximum is 256"));
}

#[test]
fn render_to_file_and_base64() {
    let h = session();
    let mut g = h.lock().unwrap();
    g.handle("doc.new", json!({"width": 40, "height": 20, "background": "black"})).unwrap();
    let b = g.handle("doc.render", json!({"maxSide": 10})).unwrap();
    let png = base64::engine::general_purpose::STANDARD.decode(b["base64"].as_str().unwrap()).unwrap();
    let img = photocraft_codecs::decode(&png).unwrap();
    assert_eq!(img.dimensions(), (10, 5));
    let path = std::env::temp_dir().join(format!("pc-rpc-{}.png", std::process::id()));
    let r = g.handle("doc.render", json!({"path": path.to_string_lossy(), "maxSide": 0})).unwrap();
    assert!(r["bytes"].as_u64().unwrap() > 0);
    assert_eq!(photocraft_codecs::decode(&std::fs::read(&path).unwrap()).unwrap().dimensions(), (40, 20));
    let _ = std::fs::remove_file(path);
}

#[test]
fn background_jobs_list_wait_and_cancel() {
    let mut h = Headless::new();
    h.handle("doc.new", json!({"width": 600, "height": 400})).unwrap();
    h.handle("engine.execute", json!({"command": "layer.new.layer"})).unwrap();
    h.handle("engine.execute", json!({"command": "edit.fill", "params": {"color": "#808080"}})).unwrap();
    // Default: waits, the result is the command's.
    let r = h.handle("engine.execute", json!({"command": "filter.noise.addNoise", "params": {"amount": 20}})).unwrap();
    assert!(r["filter"].is_object(), "{r}");
    // wait: false returns a job id at once.
    let r = h.handle("engine.execute", json!({"command": "filter.blur.gaussianBlur", "params": {"radius": 40}, "wait": false})).unwrap();
    let job = r["job"].as_u64().unwrap();
    let listed = h.handle("jobs.list", json!({})).unwrap();
    assert!(listed["jobs"].as_array().unwrap().iter().any(|j| j["id"] == job), "{listed}");
    // Wait for it via jobs.list (which applies finished jobs).
    let t = std::time::Instant::now();
    loop {
        let l = h.handle("jobs.list", json!({})).unwrap();
        let state = l["jobs"].as_array().unwrap().iter().find(|j| j["id"] == job).map(|j| j["state"].clone());
        if state == Some(json!("done")) {
            break;
        }
        assert!(t.elapsed().as_secs() < 60, "{l}");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    // Cancel: a second job, cancelled at once, leaves the document unchanged.
    let before = h.session.active().unwrap().doc.clone();
    let r = h.handle("engine.execute", json!({"command": "filter.blur.gaussianBlur", "params": {"radius": 40}, "wait": false})).unwrap();
    let job = r["job"].as_u64().unwrap();
    h.handle("jobs.cancel", json!({"job": job})).unwrap();
    assert!(h.handle("jobs.cancel", json!({"job": "nope"})).is_err());
    let l = h.handle("jobs.list", json!({})).unwrap();
    assert_eq!(l["jobs"].as_array().unwrap().iter().find(|j| j["id"] == job).unwrap()["state"], "cancelled");
    assert!(std::sync::Arc::ptr_eq(&h.session.active().unwrap().doc, &before));
}

#[test]
fn batch_steps_can_start_background_jobs() {
    let mut h = Headless::new();
    h.handle("doc.new", json!({"width": 600, "height": 400})).unwrap();
    let steps = json!([
        {"command": "layer.new.layer"},
        {"command": "edit.fill", "params": {"color": "#808080"}},
        {"command": "filter.blur.gaussianBlur", "params": {"radius": 40}, "wait": false}
    ]);
    let r = h.handle("batch", json!({"steps": steps})).unwrap();
    assert_eq!(r["completed"], 3, "{r}");
    let job = r["results"][2]["result"]["job"].as_u64().unwrap_or_else(|| panic!("no job id: {r}"));
    assert_eq!(r["results"][2]["result"]["pending"], true);
    let listed = h.handle("jobs.list", json!({})).unwrap();
    assert!(listed["jobs"].as_array().unwrap().iter().any(|j| j["id"] == job), "{listed}");
    let t = std::time::Instant::now();
    while h.session.jobs().iter().any(|j| j.id.0 == job) {
        assert!(t.elapsed().as_secs() < 60);
        std::thread::sleep(std::time::Duration::from_millis(5));
        h.handle("jobs.list", json!({})).unwrap();
    }
}

/// Regression (#503): a finished `wait: false` job was only applied by a later command or
/// `jobs.list`, so save, inspect, render and `session.list` still saw the document without it.
#[test]
fn every_request_sees_a_finished_background_job() {
    let dir = std::env::temp_dir().join(format!("pc-rpc-job-sync-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (before, after) = (dir.join("before.png"), dir.join("after.png"));
    let mut h = Headless::trusted_local();
    h.handle("doc.new", json!({"width": 64, "height": 48})).unwrap();
    h.handle("engine.execute", json!({"command": "filter.noise.addNoise", "params": {"amount": 50}})).unwrap();
    h.handle("doc.save", json!({"path": before.to_string_lossy()})).unwrap();
    let render = |h: &mut Headless| h.handle("doc.render", json!({"maxSide": 0})).unwrap()["base64"].clone();
    let unblurred = render(&mut h);
    let revision = |h: &mut Headless| h.handle("session.list", json!({})).unwrap()["documents"][0]["revision"].clone();
    let start = revision(&mut h);
    h.handle("engine.execute", json!({"command": "filter.blur.gaussianBlur", "params": {"radius": 4}, "wait": false})).unwrap();
    // Only reading requests from here on: no command and no `jobs.list`.
    let t = std::time::Instant::now();
    while revision(&mut h) == start {
        assert!(t.elapsed().as_secs() < 60, "session.list never saw the finished job");
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let history = h.handle("doc.inspect", json!({})).unwrap()["history"].clone();
    assert_eq!(history.as_array().unwrap().last().unwrap(), "Gaussian Blur", "{history}");
    h.handle("doc.save", json!({"path": after.to_string_lossy()})).unwrap();
    let decode = |p: &std::path::Path| photocraft_codecs::decode(&std::fs::read(p).unwrap()).unwrap().to_rgba8();
    assert_ne!(decode(&before), decode(&after), "the export holds the blur");
    assert_ne!(render(&mut h), unblurred, "the preview holds the blur");
    std::fs::remove_dir_all(dir).unwrap();
}

/// Regression: `quality` 85.0 or "85" was ignored (a lossless WebP, reply ok) and 0 or 500 was
/// clamped, where the CLI refuses anything but a whole number from 1 to 100.
#[test]
fn doc_save_rejects_a_bad_quality_instead_of_ignoring_or_clamping_it() {
    let dir = std::env::temp_dir().join(format!("pc-rpc-save-quality-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x.webp");
    let mut h = Headless::trusted_local();
    h.handle("doc.new", json!({"width": 16, "height": 16})).unwrap();
    for q in [json!(85.0), json!("85"), json!(0), json!(500), json!(-5), json!(true)] {
        let err = h.handle("doc.save", json!({"path": path.to_string_lossy(), "quality": q})).unwrap_err().to_string();
        assert!(err.contains("quality"), "{q}: {err}");
        assert!(!path.exists(), "{q} wrote a file");
    }
    h.handle("doc.save", json!({"path": path.to_string_lossy(), "quality": 85})).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.get(12..16), Some(&b"VP8 "[..]), "a WebP saved with a quality is lossy");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn automation_preview_rejects_oversized_full_size_and_numeric_wraparound() {
    let mut h = Headless::new();
    h.handle("doc.new", json!({"width": 2049, "height": 1})).unwrap();
    for max in [json!(0), json!(2049), json!(u64::from(u32::MAX) + 1), json!(-1), json!(1.5)] {
        assert!(h.handle("doc.render", json!({"maxSide": max})).is_err());
    }
    // A smaller preview still works, and rejection leaves the source document intact.
    let rendered = h.handle("doc.render", json!({"maxSide": 32})).unwrap();
    let png = base64::engine::general_purpose::STANDARD.decode(rendered["base64"].as_str().unwrap()).unwrap();
    assert_eq!(photocraft_codecs::decode(&png).unwrap().dimensions(), (32, 1));
    assert_eq!(h.session.active().unwrap().doc.size.width, 2049);
}

#[test]
fn stdio_rejects_oversized_frames_before_dispatch() {
    let h = Mutex::new(Headless::new());
    let input = " ".repeat(MAX_REQUEST_BYTES + 1);
    let mut out = Vec::new();
    serve_lines(&h, input.as_bytes(), &mut out).unwrap();
    let reply: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(reply["ok"], false);
    assert!(reply["error"].as_str().unwrap().contains("request exceeds"));
    assert!(h.lock().unwrap().session.documents().is_empty());
}

/// Regression (#505): an over-long or non-UTF-8 line ended the stdio session, losing every
/// open document. Now it gets one error reply, the rest of the line is skipped (never
/// dispatched), and the next request is served.
#[test]
fn stdio_skips_a_rejected_line_and_keeps_the_session() {
    let h = Mutex::new(Headless::new());
    let mut input = b"{\"id\":1,\"method\":\"doc.new\",\"params\":{\"width\":8,\"height\":8}}\n".to_vec();
    input.extend(" ".repeat(MAX_REQUEST_BYTES + 1).into_bytes());
    input.extend(b"{\"id\":9,\"method\":\"doc.close\"}\n");
    input.extend(b"\xff\xfe{\"id\":8,\"method\":\"doc.close\"}\n");
    input.extend(b"{\"id\":2,\"method\":\"session.list\"}\n");
    let mut out = Vec::new();
    serve_lines(&h, input.as_slice(), &mut out).unwrap();
    let replies: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(replies.len(), 4, "{replies:?}");
    assert_eq!(replies[0]["ok"], true);
    assert!(replies[1]["error"].as_str().unwrap().contains("request exceeds"), "{}", replies[1]);
    assert!(replies[2]["error"].as_str().unwrap().contains("not valid UTF-8"), "{}", replies[2]);
    assert_eq!(replies[3]["id"], 2);
    assert_eq!(replies[3]["result"]["documents"].as_array().unwrap().len(), 1, "the document stays open");
}

#[test]
fn preview_rejection_precedes_output_file_creation() {
    let root = std::env::temp_dir().join(format!("pc-preview-budget-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let workspace = crate::AuthorizedWorkspace::new(None, Some(&root)).unwrap();
    let mut h = Headless::with_workspace(workspace);
    h.handle("doc.new", json!({"width": 16, "height": 8})).unwrap();
    let result = h.handle("doc.render", json!({"maxSide": 2049, "path": "rejected.png"}));
    assert!(result.unwrap_err().to_string().contains("preview side exceeds"));
    assert!(!root.join("rejected.png").exists());
    assert!(h.handle("doc.render", json!({"maxSide": 16, "path": "accepted.png"})).is_ok());
    assert!(root.join("accepted.png").exists());
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_file(root.join("accepted.png")).unwrap();
    std::fs::remove_dir(root).unwrap();
}

#[test]
fn doc_place_reads_through_the_read_root_and_never_outside_it() {
    let base = std::env::temp_dir().join(format!("pc-doc-place-{}", std::process::id()));
    let root = base.join("workspace");
    let outside = base.join("outside");
    for dir in [&root, &outside] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let img = photocraft_codecs::Image::from_u8(8, 6, photocraft_codecs::ChannelLayout::Rgb, vec![42; 8 * 6 * 3]).unwrap();
    let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    std::fs::write(root.join("logo.png"), &png).unwrap();
    std::fs::write(outside.join("logo.png"), &png).unwrap();
    let absolute = outside.join("logo.png").to_string_lossy().into_owned();

    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.new", json!({"width": 40, "height": 30})).unwrap();
    let placed = h.handle("doc.place", json!({"path": "logo.png", "scale": 50})).unwrap();
    assert!(placed["layer"].as_u64().is_some(), "{placed}");
    assert_eq!(placed["scale"].as_f64(), Some(50.0), "the options reach the command: {placed}");
    let inspected = h.handle("doc.inspect", json!({})).unwrap();
    assert_eq!(inspected["layers"].as_array().map(Vec::len), Some(2), "the placed layer is in the document: {inspected}");
    // A path outside the read root is refused the way the ambient command is (#2810).
    for (path, reason) in [("../outside/logo.png", "parent traversal"), (absolute.as_str(), "must be inside the automation root")] {
        let refused = h.handle("doc.place", json!({"path": path})).unwrap_err().to_string();
        assert!(refused.contains(reason), "{refused}");
        let inspected = h.handle("doc.inspect", json!({})).unwrap();
        assert_eq!(inspected["layers"].as_array().map(Vec::len), Some(2), "nothing was placed: {refused}");
    }
    // Trusted local keeps the ambient command: that caller may read the path itself.
    let mut t = Headless::trusted_local();
    t.handle("doc.new", json!({"width": 40, "height": 30})).unwrap();
    let trusted = t.handle("doc.place", json!({"path": root.join("logo.png").to_string_lossy()})).unwrap();
    assert!(trusted["layer"].as_u64().is_some(), "{trusted}");
    // Release directory capabilities before cleanup on Windows.
    drop(t);
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

/// A save or render never turns a file PhotoCraft doesn't write (source, settings, notes)
/// into image bytes, whatever `format` says; images and new files are written as before (#9439).
#[test]
fn workspace_writes_refuse_to_replace_a_file_that_is_not_an_image() {
    let root = std::env::temp_dir().join(format!("pc-no-replace-foreign-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    let foreign = [("src/main.rs", "fn main() {}\n"), ("Cargo.toml", "[package]\n"), (".env", "TOKEN=x\n"), ("Makefile", "all:\n")];
    for (name, text) in foreign {
        std::fs::write(root.join(name), text).unwrap();
    }
    std::fs::write(root.join("old.png"), b"not yet a png").unwrap();
    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    for (name, text) in foreign {
        let saved = h.handle("doc.save", json!({"path": name, "format": "png"})).unwrap_err().to_string();
        assert!(saved.contains("already exists"), "doc.save over {name}: {saved}");
        let rendered = h.handle("doc.render", json!({"path": name})).unwrap_err().to_string();
        assert!(rendered.contains("already exists"), "doc.render over {name}: {rendered}");
        assert_eq!(std::fs::read_to_string(root.join(name)).unwrap(), text, "{name} is untouched");
    }
    // An existing image is replaced, and a new file of any name is created.
    h.handle("doc.save", json!({"path": "old.png"})).unwrap();
    assert!(std::fs::read(root.join("old.png")).unwrap().starts_with(b"\x89PNG"));
    h.handle("doc.save", json!({"path": "fresh.txt", "format": "png"})).unwrap();
    assert!(std::fs::read(root.join("fresh.txt")).unwrap().starts_with(b"\x89PNG"));
    h.handle("doc.render", json!({"path": "old.png"})).unwrap();
    std::fs::write(root.join("OLD.TIF"), b"x").unwrap();
    h.handle("doc.save", json!({"path": "OLD.TIF"})).unwrap();
    // An image saved under a name without an image extension is recognised by its bytes.
    h.handle("doc.save", json!({"path": "out", "format": "png"})).unwrap();
    h.handle("doc.save", json!({"path": "out", "format": "png"})).unwrap();
    h.handle("doc.render", json!({"path": "fresh.txt"})).unwrap();
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn doc_replace_smart_object_reads_through_the_read_root_and_never_outside_it() {
    let base = std::env::temp_dir().join(format!("pc-doc-replace-so-{}", std::process::id()));
    let root = base.join("workspace");
    let outside = base.join("outside");
    for dir in [&root, &outside] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let png = |w: u32, h: u32, v: u8| {
        let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgb, vec![v; (w * h * 3) as usize]).unwrap();
        photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()
    };
    std::fs::write(root.join("logo.png"), png(8, 6, 42)).unwrap();
    std::fs::write(root.join("red.png"), png(8, 6, 200)).unwrap();
    std::fs::write(outside.join("red.png"), png(8, 6, 200)).unwrap();
    let absolute = outside.join("red.png").to_string_lossy().into_owned();

    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.new", json!({"width": 40, "height": 30})).unwrap();
    let layer = h.handle("doc.place", json!({"path": "logo.png"})).unwrap()["layer"].clone();
    // The smart object's file name and the composite at its centre: what Replace Contents changes.
    let contents = |h: &mut Headless| {
        let inspected = h.handle("doc.inspect", json!({})).unwrap();
        let layers = inspected["layers"].as_array().unwrap();
        let so = layers.iter().find(|l| l["id"] == layer).unwrap_or_else(|| panic!("{inspected}"));
        let name = so["smartSource"]["fileName"].as_str().unwrap_or_default().to_owned();
        let rendered = h.handle("doc.render", json!({"maxSide": 0})).unwrap();
        let png = base64::engine::general_purpose::STANDARD.decode(rendered["base64"].as_str().unwrap()).unwrap();
        let img = photocraft_codecs::decode(&png).unwrap();
        let (w, ht) = img.dimensions();
        let data = img.as_u8().unwrap();
        let channels = data.len() / (w * ht) as usize;
        (name, data[((ht / 2 * w + w / 2) as usize) * channels])
    };
    assert_eq!(contents(&mut h), ("logo.png".to_owned(), 42), "the placed contents");
    let replaced = h.handle("doc.replaceSmartObject", json!({"path": "red.png", "layer": layer})).unwrap();
    assert_eq!(replaced["layer"], layer, "{replaced}");
    assert_eq!(contents(&mut h), ("red.png".to_owned(), 200), "the contents were replaced");
    h.handle("engine.execute", json!({"command": "edit.undo"})).unwrap();
    assert_eq!(contents(&mut h), ("logo.png".to_owned(), 42), "one undo step restores the old contents");
    h.handle("engine.execute", json!({"command": "edit.redo"})).unwrap();
    assert_eq!(contents(&mut h), ("red.png".to_owned(), 200), "redo replaces them again");
    // Outside the read root, a bad layer, a missing path or a file that is no image: refused, nothing changed.
    for (params, reason) in [
        (json!({"path": "../outside/red.png"}), "parent traversal"),
        (json!({"path": absolute}), "must be inside the automation root"),
        (json!({"path": "logo.png", "layer": 999_999}), "no such layer"),
        (json!({}), "needs `path`"),
    ] {
        let refused = h.handle("doc.replaceSmartObject", params).unwrap_err().to_string();
        assert!(refused.contains(reason), "{refused}");
        assert_eq!(contents(&mut h), ("red.png".to_owned(), 200), "nothing changed: {refused}");
    }
    std::fs::write(root.join("junk.png"), b"not an image").unwrap();
    assert!(h.handle("doc.replaceSmartObject", json!({"path": "junk.png", "layer": layer})).is_err());
    assert_eq!(contents(&mut h), ("red.png".to_owned(), 200), "a file that is no image changes nothing");
    // The ambient command keeps its refusal.
    let ambient = h.handle("engine.execute", json!({"command": "layer.smartObjects.replaceContents", "params": {"path": "red.png"}}));
    assert!(ambient.unwrap_err().to_string().contains("ambient filesystem paths"));
    // Trusted local keeps the ambient command.
    let mut t = Headless::trusted_local();
    t.handle("doc.new", json!({"width": 40, "height": 30})).unwrap();
    let tl = t.handle("doc.place", json!({"path": root.join("logo.png").to_string_lossy()})).unwrap()["layer"].clone();
    let trusted = t.handle("doc.replaceSmartObject", json!({"path": root.join("red.png").to_string_lossy(), "layer": tl})).unwrap();
    assert_eq!(trusted["layer"], tl, "{trusted}");
    drop(t);
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn doc_export_layers_reports_skips_in_workspace_and_trusted_sessions() {
    for trusted in [false, true] {
        let base = std::env::temp_dir().join(format!("pc-export-skips-{}-{trusted}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut h =
            if trusted { Headless::trusted_local() } else { Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&base), Some(&base)).unwrap()) };
        h.handle("doc.new", json!({"width": 4, "height": 4, "background": "white", "name": "card"})).unwrap();
        let adjustment = h.handle("engine.execute", json!({"command": "layer.newAdjustmentLayer.levels"})).unwrap()["layer"].clone();
        h.handle("engine.execute", json!({"command": "layer.setProps", "params": {"name": "Levels"}})).unwrap();
        let hidden = h.handle("engine.execute", json!({"command": "layer.new.layer", "params": {"name": "Hidden"}})).unwrap()["layer"].clone();
        h.handle("engine.execute", json!({"command": "layer.setProps", "params": {"visible": false}})).unwrap();
        // Exercise the actual JSON-lines serve envelope, not only direct dispatch.
        let h = Mutex::new(h);
        let out_dir = base.join("out").to_string_lossy().into_owned();
        let request = json!({"id": 8491, "method": "doc.exportLayers", "params": {"dir": out_dir}}).to_string() + "\n";
        let mut output = Vec::new();
        serve_lines(&h, request.as_bytes(), &mut output).unwrap();
        let reply: Value = serde_json::from_slice(&output).unwrap();
        assert_eq!(reply["id"], 8491);
        assert_eq!(reply["ok"], true, "{reply}");
        assert_eq!(reply["result"]["files"], json!([format!("{out_dir}/card_0001_Background.png")]));
        if !trusted {
            assert_eq!(reply["result"]["count"], 1);
        }
        assert_eq!(
            reply["result"]["skipped"],
            json!([
                {"layer": hidden, "name": "Hidden", "reason": "hidden by visibleOnly"},
                {"layer": adjustment, "name": "Levels", "reason": "adjustment layer has no pixels of its own"}
            ])
        );
        assert_eq!(std::fs::read_dir(base.join("out")).unwrap().count(), 1);
        let mut h = h.into_inner().unwrap();
        let all_dir = base.join("all").to_string_lossy().into_owned();
        let all = h.export_layers(None, &all_dir, &json!({"visibleOnly": false})).unwrap();
        assert_eq!(all["files"].as_array().unwrap().len(), 2);
        assert_eq!(all["skipped"].as_array().unwrap().len(), 1);
        // Workspace no-overwrite errors still fail and remove only newly created files.
        if !trusted {
            let collision = base.join("collision");
            std::fs::create_dir_all(&collision).unwrap();
            std::fs::write(collision.join("card_0002_Background.png"), b"keep").unwrap();
            assert!(h.export_layers(None, &collision.to_string_lossy(), &json!({"visibleOnly": false})).is_err());
            assert_eq!(std::fs::read_dir(&collision).unwrap().count(), 1);
            assert_eq!(std::fs::read(collision.join("card_0002_Background.png")).unwrap(), b"keep");
        }
        drop(h);
        std::fs::remove_dir_all(base).unwrap();
    }
}

#[test]
fn doc_export_layers_writes_only_beneath_the_write_root() {
    let base = std::env::temp_dir().join(format!("pc-doc-export-layers-{}", std::process::id()));
    let root = base.join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.new", json!({"width": 12, "height": 8, "background": "white", "name": "card"})).unwrap();
    h.handle("engine.execute", json!({"command": "layer.new.layer", "params": {"name": "Badge"}})).unwrap();

    let out = root.join("out");
    let exported = h.handle("doc.exportLayers", json!({"dir": out.to_string_lossy(), "format": "png", "prefix": "card"})).unwrap();
    assert_eq!(exported["count"].as_u64(), Some(2), "{exported}");
    let mut names: Vec<String> = std::fs::read_dir(&out).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert_eq!(names.len(), 2, "{exported}");
    for name in names {
        assert!(name.starts_with("card_") && name.ends_with(".png"), "{name}");
        let bytes = std::fs::read(out.join(&name)).unwrap();
        assert_eq!(photocraft_codecs::decode(&bytes).unwrap().dimensions(), (12, 8), "{name}");
    }
    // Outside the write root nothing is created, not even the directory.
    let escaped = base.join("escaped");
    let refused = h.handle("doc.exportLayers", json!({"dir": escaped.to_string_lossy()})).unwrap_err().to_string();
    assert!(refused.contains("must be inside the automation root"), "{refused}");
    assert!(!escaped.exists());
    // The ambient command keeps its refusal for the very same directory.
    let ambient =
        h.handle("engine.execute", json!({"command": "file.export.layersToFiles", "params": {"dir": out.to_string_lossy()}})).unwrap_err().to_string();
    assert!(ambient.contains("ambient filesystem paths"), "{ambient}");
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn doc_export_data_sets_refuse_collisions_and_import_reads_the_root() {
    let base = std::env::temp_dir().join(format!("pc-doc-data-sets-{}", std::process::id()));
    let root = base.join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.new", json!({"width": 8, "height": 8, "background": "white", "name": "card"})).unwrap();
    let inspected = h.handle("doc.inspect", json!({})).unwrap();
    let layer = inspected["layers"][0]["id"].as_u64().expect("the layer id");
    h.handle("engine.execute", json!({"command": "image.variables.define", "params": {"defs": [{"name": "show", "layer": layer, "type": "visibility"}]}}))
        .unwrap();
    h.handle(
        "engine.execute",
        json!({"command": "image.variables.dataSets", "params": {"dataSets": [
            {"name": "one", "values": [{"variable": "show", "kind": "visibility", "value": true}]},
            {"name": "two", "values": [{"variable": "show", "kind": "visibility", "value": false}]},
        ]}}),
    )
    .unwrap();

    let sets = root.join("sets");
    let exported = h.handle("doc.exportDataSets", json!({"dir": sets.to_string_lossy(), "format": "png"})).unwrap();
    assert_eq!(exported["count"].as_u64(), Some(2), "{exported}");
    assert_eq!(std::fs::read_dir(&sets).unwrap().count(), 2, "{exported}");
    // Colliding names are refused as in the command, before any file is written.
    let clash = root.join("clash");
    let refused = h.handle("doc.exportDataSets", json!({"dir": clash.to_string_lossy(), "naming": "{document}"})).unwrap_err().to_string();
    assert!(refused.contains("{index}"), "{refused}");
    assert_eq!(std::fs::read_dir(&clash).unwrap().count(), 0, "no file of the refused batch");
    // A CSV from the read root, and one outside it.
    std::fs::write(root.join("sets.csv"), "Data Set,show\none,true\ntwo,false\n").unwrap();
    let imported = h.handle("doc.importDataSets", json!({"path": "sets.csv"})).unwrap();
    assert_eq!(imported["imported"].as_u64(), Some(2), "{imported}");
    std::fs::write(base.join("outside.csv"), "Data Set,show\none,true\n").unwrap();
    let outside = h.handle("doc.importDataSets", json!({"path": base.join("outside.csv").to_string_lossy()})).unwrap_err().to_string();
    assert!(outside.contains("must be inside the automation root"), "{outside}");
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn indexed_place_replace_and_data_set_methods_keep_the_active_document() {
    let base = std::env::temp_dir().join(format!("pc-doc-index-active-{}", std::process::id()));
    let root = base.join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let img = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgb, vec![42; 4 * 4 * 3]).unwrap();
    std::fs::write(root.join("logo.png"), photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
    std::fs::write(root.join("sets.csv"), "Data Set,show\none,true\ntwo,false\n").unwrap();
    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.new", json!({"width": 8, "height": 8, "background": "white", "name": "first"})).unwrap();
    h.handle("doc.new", json!({"width": 8, "height": 8, "background": "white", "name": "second"})).unwrap();
    let layer = h.handle("doc.inspect", json!({})).unwrap()["layers"][0]["id"].as_u64().expect("the layer id");
    h.handle("engine.execute", json!({"command": "image.variables.define", "params": {"defs": [{"name": "show", "layer": layer, "type": "visibility"}]}}))
        .unwrap();
    h.handle("doc.select", json!({"index": 0})).unwrap();

    let steps = [
        ("doc.place", json!({"path": "logo.png", "index": 1}), true),
        // Document 1's active layer is now the placed Smart Object.
        ("doc.replaceSmartObject", json!({"path": "logo.png", "index": 1}), true),
        ("doc.replaceSmartObject", json!({"path": "missing.png", "index": 1}), false),
        ("doc.importDataSets", json!({"path": "sets.csv", "index": 1}), true),
        ("doc.exportLayers", json!({"dir": "layers", "index": 1}), true),
        ("doc.exportDataSets", json!({"dir": "sets", "index": 1}), true),
        // A failing call restores the active document too.
        ("doc.importDataSets", json!({"path": "missing.csv", "index": 1}), false),
    ];
    for (method, params, succeeds) in steps {
        let result = h.handle(method, params.clone());
        assert_eq!(result.is_ok(), succeeds, "{method} {params}: {result:?}");
        assert_eq!(h.session.active_index(), Some(0), "{method} {params} must not select document 1");
    }
    assert_eq!(h.session.documents()[0].doc.layer_count(), 1, "nothing was placed into the active document");
    assert_eq!(h.session.documents()[1].doc.layer_count(), 2, "the placed layer is in document 1");
    assert_eq!(std::fs::read_dir(root.join("sets")).unwrap().count(), 2, "document 1's data sets were exported");
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn doc_export_data_sets_judges_pixel_replacement_paths_like_apply() {
    let base = std::env::temp_dir().join(format!("pc-doc-data-set-pixels-{}", std::process::id()));
    let root = base.join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let red = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgb, [255, 0, 0].repeat(16)).unwrap();
    let photo = base.join("photo.png");
    std::fs::write(&photo, photocraft_codecs::encode(&red, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.new", json!({"width": 8, "height": 8, "background": "white", "name": "card"})).unwrap();
    let layer = h.handle("engine.execute", json!({"command": "layer.new.layer", "params": {}})).unwrap()["layer"].clone();
    h.handle(
        "engine.execute",
        json!({"command": "image.variables.define", "params": {"defs": [{"name": "photo", "layer": layer, "type": "pixelReplacement"}]}}),
    )
    .unwrap();
    // Storing the path is allowed; reading it is what the gate judges.
    h.handle(
        "engine.execute",
        json!({"command": "image.variables.dataSets", "params": {"dataSets": [
            {"name": "swap", "values": [{"variable": "photo", "kind": "pixels", "value": photo.to_string_lossy()}]},
        ]}}),
    )
    .unwrap();
    let applied = h.handle("engine.execute", json!({"command": "image.applyDataSet", "params": {"name": "swap"}})).unwrap_err().to_string();
    assert!(applied.contains("ambient filesystem paths"), "{applied}");
    let sets = root.join("sets");
    let exported = h.handle("doc.exportDataSets", json!({"dir": sets.to_string_lossy(), "format": "png"})).unwrap_err().to_string();
    assert!(exported.contains("ambient filesystem paths"), "{exported}");
    assert_eq!(std::fs::read_dir(&sets).map_or(0, Iterator::count), 0, "nothing of the refused batch is written");
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn workspace_psd_saves_embed_no_file_a_linked_smart_object_names() {
    use photocraft_engine::doc::{LayerContent, SmartSource};

    // An image PhotoCraft can open (other linked files are never embedded): an embedded copy
    // carries these exact bytes, while pixels written for the layer don't.
    let image = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgb, (0..192u8).collect()).unwrap();
    let marker = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let has_marker = |bytes: &[u8]| bytes.windows(marker.len()).any(|w| w == marker.as_slice());
    let base = std::env::temp_dir().join(format!("pc-psd-linked-embed-{}", std::process::id()));
    let root = base.join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let outside = base.join("outside.png");
    std::fs::write(&outside, &marker).unwrap();
    // A document whose smart object names a file outside the roots as its source.
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let state = session.active_mut().unwrap();
    let LayerContent::Smart(smart) = &mut std::sync::Arc::make_mut(&mut state.doc).layers[0].content else { panic!("smart object") };
    smart.source = SmartSource::Linked { path: outside.to_string_lossy().into_owned() };
    std::fs::write(root.join("in.pcraft"), photocraft_engine::smart_cmds::encode_source(&state.doc).unwrap()).unwrap();
    // A trusted export embeds the linked file, as before.
    assert!(has_marker(&photocraft_io::export(&state.doc, "psd", &Default::default()).unwrap().bytes));

    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.open", json!({"path": "in.pcraft"})).unwrap();
    let saved = h.handle("doc.save", json!({"path": "out.psd"})).unwrap();
    let warnings = saved["warnings"].to_string();
    assert!(warnings.contains("written as pixels"), "{warnings}");
    assert!(!has_marker(&std::fs::read(root.join("out.psd")).unwrap()), "the linked file outside the roots is not embedded");
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn allowed_filters_do_not_rerender_a_smart_object_from_its_linked_file() {
    use photocraft_engine::doc::{LayerContent, SmartSource};

    let base = std::env::temp_dir().join(format!("pc-linked-rerender-{}", std::process::id()));
    let root = base.join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let red = photocraft_codecs::Image::from_u8(8, 8, photocraft_codecs::ChannelLayout::Rgb, [255, 0, 0].repeat(64)).unwrap();
    let outside = base.join("red.png");
    std::fs::write(&outside, photocraft_codecs::encode(&red, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
    // A document whose white smart object names a file outside the roots as its source.
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let state = session.active_mut().unwrap();
    let LayerContent::Smart(smart) = &mut std::sync::Arc::make_mut(&mut state.doc).layers[0].content else { panic!("smart object") };
    smart.source = SmartSource::Linked { path: outside.to_string_lossy().into_owned() };
    std::fs::write(root.join("p.pcraft"), photocraft_engine::smart_cmds::encode_source(&state.doc).unwrap()).unwrap();

    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.open", json!({"path": "p.pcraft"})).unwrap();
    h.handle("engine.execute", json!({"command": "filter.blur.gaussianBlur", "params": {"radius": 0.5}})).unwrap();
    let doc = &h.session.active().unwrap().doc;
    let LayerContent::Smart(smart) = &doc.layers[0].content else { panic!("smart object") };
    assert_eq!(smart.smart_filters.len(), 1, "the filter is still recorded");
    assert_eq!(smart.cache.as_ref().unwrap().rgba(4, 4), [1.0, 1.0, 1.0, 1.0], "the linked file outside the roots is not read");
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn workspace_layer_exports_keep_linked_smart_objects_as_pixels() {
    use photocraft_engine::doc::{LayerContent, SmartSource};

    const MARKER: &[u8] = b"PHOTOCRAFT-LINKED-0002";
    let has_marker = |bytes: &[u8]| bytes.windows(MARKER.len()).any(|w| w == MARKER);
    let base = std::env::temp_dir().join(format!("pc-export-layers-linked-{}", std::process::id()));
    let root = base.join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    let outside = base.join("outside.txt");
    std::fs::write(&outside, MARKER).unwrap();
    // A document whose smart object names a file outside the roots as its source.
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 8, "height": 8, "background": "white"})).unwrap();
    session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let state = session.active_mut().unwrap();
    let LayerContent::Smart(smart) = &mut std::sync::Arc::make_mut(&mut state.doc).layers[0].content else { panic!("smart object") };
    smart.source = SmartSource::Linked { path: outside.to_string_lossy().into_owned() };
    std::fs::write(root.join("in.pcraft"), photocraft_engine::smart_cmds::encode_source(&state.doc).unwrap()).unwrap();

    let mut h = Headless::with_workspace(crate::AuthorizedWorkspace::new(Some(&root), Some(&root)).unwrap());
    h.handle("doc.open", json!({"path": "in.pcraft"})).unwrap();
    let out = root.join("layers");
    for format in ["psd", "psb"] {
        let exported = h.handle("doc.exportLayers", json!({"dir": out.to_string_lossy(), "format": format})).unwrap();
        assert_eq!(exported["count"], 1, "{exported}");
    }
    let files: Vec<_> = std::fs::read_dir(&out).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(files.len(), 2, "{files:?}");
    for file in files {
        assert!(!has_marker(&std::fs::read(&file).unwrap()), "{} embeds no file outside the roots", file.display());
    }
    // Release directory capabilities before cleanup on Windows.
    drop(h);
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn trusted_local_render_keeps_full_size_behavior() {
    let mut h = Headless::trusted_local();
    h.handle("doc.new", json!({"width": 2049, "height": 1})).unwrap();
    let rendered = h.handle("doc.render", json!({"maxSide": 0})).unwrap();
    let png = base64::engine::general_purpose::STANDARD.decode(rendered["base64"].as_str().unwrap()).unwrap();
    assert_eq!(photocraft_codecs::decode(&png).unwrap().dimensions(), (2049, 1));
}

#[test]
fn methods_lists_the_job_methods_and_every_listed_name_is_served() {
    let mut h = Headless::new();
    let listed = h.handle("methods", Value::Null).unwrap();
    let listed: Vec<&str> = listed.as_array().unwrap().iter().map(|m| m.as_str().unwrap()).collect();
    assert_eq!(listed, METHODS);
    // The job methods `engine.execute {"wait": false}` depends on are discoverable (#414).
    assert!(listed.contains(&"jobs.list") && listed.contains(&"jobs.cancel"));
    for m in METHODS {
        if let Err(e) = h.handle(m, Value::Null) {
            assert!(!e.to_string().contains("unknown method"), "{m} is listed but not served: {e}");
        }
    }
    assert!(h.handle("jobs.nope", Value::Null).unwrap_err().to_string().contains("unknown method"));
}

#[test]
fn batch_response_budget_stops_later_steps_even_when_stop_on_error_is_false() {
    let mut h = Headless::new();
    let mut steps = vec![json!({"command": "command.list"}); MAX_BATCH_STEPS - 1];
    steps.push(json!({"command": "file.new", "params": {"width": 8, "height": 8}}));
    let result = h.batch(&json!({"steps": steps, "stopOnError": false})).unwrap();
    assert_eq!(result["failed"], 1);
    assert!(result["results"].as_array().unwrap().last().unwrap()["error"].as_str().unwrap().contains("batch response budget exceeded"));
    assert!(h.session.documents().is_empty(), "later edit must not execute after the quota is exhausted");
    assert!(crate::budgets::json_bytes(&result).is_ok());
    assert!(h.handle("methods", Value::Null).is_ok());
}

#[test]
fn idle_tcp_daemon_releases_its_port_but_keeps_active_clients() {
    use std::io::BufReader;
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let h = Arc::new(Mutex::new(Headless::new()));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let result = serve_tcp_until_idle("127.0.0.1:0", h, TOKEN.to_owned(), move |addr| ready_tx.send(addr).unwrap(), Duration::from_millis(200));
        done_tx.send(result).unwrap();
    });
    let addr = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let mut client = std::net::TcpStream::connect(addr).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut reader = BufReader::new(client.try_clone().unwrap());
    writeln!(client, r#"{{"id":"auth","method":"auth","params":{{"token":"{TOKEN}"}}}}"#).unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let auth: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(auth["result"]["authenticated"], true);
    // The port stays owned well past the idle limit while a client is connected.
    assert!(matches!(done_rx.recv_timeout(Duration::from_millis(450)), Err(std::sync::mpsc::RecvTimeoutError::Timeout)), "daemon exited with a live client");
    drop(reader);
    drop(client);
    assert!(done_rx.recv_timeout(Duration::from_secs(3)).unwrap().is_ok(), "idle daemon did not exit cleanly");
    server.join().unwrap();
    assert!(TcpListener::bind(addr).is_ok(), "idle listener still owns its port");
}

#[test]
fn tcp_serves_loopback() {
    use std::io::{BufReader, Write as _};
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let h = Arc::new(Mutex::new(Headless::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = serve_tcp("127.0.0.1:0", h, TOKEN.into(), move |a| tx.send(a).unwrap());
    });
    let addr = rx.recv().unwrap();
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    writeln!(s, r#"{{"id":"auth","method":"auth","params":{{"token":"{TOKEN}"}}}}"#).unwrap();
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let auth: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(auth["result"]["authenticated"], true);
    writeln!(s, r#"{{"id":"a","method":"methods"}}"#).unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["id"], "a");
    assert!(v["result"].as_array().unwrap().iter().any(|m| m == "batch"));
}

/// #4965: a connection that keeps sending without authenticating is closed at the deadline,
/// and one that authenticated in time keeps being served after it.
#[test]
fn tcp_connections_must_authenticate_before_the_deadline_whatever_they_send() {
    use std::io::{BufReader, Write as _};
    use std::time::{Duration, Instant};
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let h = Mutex::new(Headless::new());
        let mut results = Vec::new();
        for _ in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            let started = Instant::now();
            let r = serve_tcp_connection(&h, stream, TOKEN, Duration::from_millis(300));
            results.push((r.map_err(|e| e.kind()), started.elapsed()));
        }
        results
    });

    // Never a newline, a byte every 20 ms: each read succeeds long before the idle timeout.
    let mut slow = std::net::TcpStream::connect(addr).unwrap();
    for _ in 0..150 {
        if slow.write_all(b" ").is_err() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(slow);

    // Authenticates at once, then asks again 300 ms after the deadline.
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    writeln!(s, r#"{{"id":"auth","method":"auth","params":{{"token":"{TOKEN}"}}}}"#).unwrap();
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["result"]["authenticated"], true);
    std::thread::sleep(Duration::from_millis(600));
    writeln!(s, r#"{{"id":"a","method":"methods"}}"#).unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&line).unwrap()["id"], "a");
    drop(reader);
    drop(s);

    let results = server.join().unwrap();
    let (slow_result, slow_time) = &results[0];
    assert!(matches!(slow_result, Err(std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)), "{slow_result:?}");
    assert!(*slow_time < Duration::from_secs(3), "the trickle did not extend the deadline: {slow_time:?}");
    assert_eq!(results[1].0, Ok(()), "the authenticated connection ended with its client");
}

#[test]
fn tcp_repeat_auth_reports_already_authenticated_and_keeps_serving() {
    use std::io::{BufReader, Write as _};
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let h = Arc::new(Mutex::new(Headless::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = serve_tcp("127.0.0.1:0", h, TOKEN.into(), move |a| tx.send(a).unwrap());
    });
    let addr = rx.recv().unwrap();
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut line = String::new();
    for id in ["auth", "again"] {
        writeln!(s, r#"{{"id":"{id}","method":"auth","params":{{"token":"{TOKEN}"}}}}"#).unwrap();
        line.clear();
        reader.read_line(&mut line).unwrap();
    }
    let again: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(again["id"], "again");
    assert_eq!(again["ok"], false);
    assert_eq!(again["error"], "already authenticated");
    writeln!(s, r#"{{"id":"m","method":"methods"}}"#).unwrap();
    line.clear();
    reader.read_line(&mut line).unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["id"], "m");
    assert_eq!(v["ok"], true);
}

#[test]
fn tcp_rejects_requests_before_authentication() {
    use std::io::{BufReader, Write as _};
    const TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let h = Arc::new(Mutex::new(Headless::new()));
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = serve_tcp("127.0.0.1:0", h, TOKEN.into(), move |a| tx.send(a).unwrap());
    });
    let addr = rx.recv().unwrap();
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    writeln!(s, r#"{{"id":1,"method":"methods"}}"#).unwrap();
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).unwrap();
    let v: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["error"], "authentication required");
    assert!(v.get("result").is_none());
}

#[test]
fn invalid_document_index_is_rejected_not_the_active_document() {
    let h = session();
    for _ in 0..2 {
        assert_eq!(respond(&h, r#"{"method":"doc.new","params":{"width":8,"height":8}}"#)["ok"], true);
    }
    let count = |h: &Mutex<Headless>| h.lock().unwrap().session_list()["documents"].as_array().unwrap().len();
    for index in [json!("0"), json!(-1), json!(0.5), json!(true)] {
        for method in ["doc.close", "doc.inspect", "doc.select"] {
            let reply = respond(&h, &json!({"method": method, "params": {"index": index}}).to_string());
            assert_eq!(reply["ok"], false, "{method} {index}: {reply}");
            assert!(reply["error"].as_str().unwrap().contains("`index` must be a non-negative integer"), "{reply}");
        }
        assert_eq!(count(&h), 2, "index {index} closed a document");
    }
    // Null means "the active document", like a missing `index`.
    assert_eq!(respond(&h, r#"{"method":"doc.inspect","params":{"index":null}}"#)["ok"], true);
    // A valid index still closes that document.
    let reply = respond(&h, r#"{"method":"doc.close","params":{"index":0}}"#);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(count(&h), 1);
}
