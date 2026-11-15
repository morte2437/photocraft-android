//! Tests for photocraft-cli GPU rendering flags, precedence, and fallback behavior.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn bin() -> Command {
    let mut cmd = crate::command();
    cmd.env_remove("PHOTOCRAFT_CONTROL_PORT");
    cmd.env_remove("PHOTOCRAFT_CONTROL_TOKEN");
    cmd.env_remove("PHOTOCRAFT_CONTROL_TOKEN_FILE");
    cmd.env_remove("PHOTOCRAFT_GPU");
    cmd
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pc-cli-gpu-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_png(path: &Path, w: u32, h: u32) {
    let data: Vec<u8> = (0..w * h * 4).map(|i| (i as u8).wrapping_mul(7)).collect();
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, data).unwrap();
    std::fs::write(path, photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
}

#[test]
fn gpu_and_no_gpu_conflict() {
    let d = tmp("conflict");
    let in_png = d.join("in.png");
    let out_png = d.join("out.png");
    write_png(&in_png, 16, 16);

    let o = bin().args(["convert", "--gpu", "--no-gpu", in_png.to_str().unwrap(), out_png.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("cannot be used with") || stderr.contains("conflicts with"), "{stderr}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn gpu_env_var_validation() {
    let d = tmp("env_val");
    let in_png = d.join("in.png");
    let out_png = d.join("out.png");
    write_png(&in_png, 16, 16);

    // Invalid non-empty value rejected with usage error naming the variable
    for bad in ["2", "true", "yes", "false", "on", "off"] {
        let o = bin().env("PHOTOCRAFT_GPU", bad).args(["convert", in_png.to_str().unwrap(), out_png.to_str().unwrap()]).output().unwrap();
        assert_eq!(o.status.code(), Some(2), "expected exit 2 for PHOTOCRAFT_GPU={bad}");
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert!(stderr.contains("PHOTOCRAFT_GPU"), "expected stderr to name variable: {stderr}");
        assert!(stderr.contains("expected '1' or '0'"), "expected usage note: {stderr}");
    }

    // Empty or blank value is accepted as unset (CPU default)
    for empty in ["", "   "] {
        let out_sub = d.join(format!("out_{}.png", empty.trim()));
        let o = bin().env("PHOTOCRAFT_GPU", empty).args(["convert", in_png.to_str().unwrap(), out_sub.to_str().unwrap()]).output().unwrap();
        assert!(o.status.success(), "expected success for PHOTOCRAFT_GPU={empty:?}");
        assert!(out_sub.is_file());
    }

    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn gpu_env_var_rejects_a_non_utf8_value() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let d = tmp("env_non_utf8");
    let in_png = d.join("in.png");
    let out_png = d.join("out.png");
    write_png(&in_png, 16, 16);

    let bad = OsString::from_vec(vec![0xff, 0xfe]);
    let o = bin().env("PHOTOCRAFT_GPU", bad).args(["convert", in_png.to_str().unwrap(), out_png.to_str().unwrap()]).output().unwrap();
    assert_eq!(o.status.code(), Some(2), "a non-UTF-8 PHOTOCRAFT_GPU must be a usage error");
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("PHOTOCRAFT_GPU"), "the usage error must name the variable: {stderr}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn explicit_flags_override_environment() {
    let d = tmp("override");
    let in_png = d.join("in.png");
    let in_layers = d.join("in.pcraft");
    let out1 = d.join("out1.png");
    let out2 = d.join("out2.png");
    write_png(&in_png, 16, 16);
    // A single-layer PNG takes the native export path and never calls the renderer; a
    // multi-layer document composites, so it observes whether `--gpu` reached the backend.
    let o = bin().args(["run", "--new", r#"{"width":16,"height":16}"#, "--cmd", "layer.new.layer", "--out", in_layers.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));

    // --no-gpu overrides PHOTOCRAFT_GPU=1
    let o = bin().env("PHOTOCRAFT_GPU", "1").args(["convert", "--no-gpu", in_png.to_str().unwrap(), out1.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out1.is_file());

    // --gpu overrides PHOTOCRAFT_GPU=0
    let o = bin().env("PHOTOCRAFT_GPU", "0").args(["convert", "--gpu", in_layers.to_str().unwrap(), out2.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out2.is_file());
    let stderr = String::from_utf8_lossy(&o.stderr);
    // The renderer reports either its adapter, an unavailable adapter, or (in a build without
    // GPU support) that GPU rendering was requested; the CPU default emits none of these.
    let markers = ["GPU adapter:", "no GPU adapter available", "could not build GPU compositor", "falling back to CPU"];
    assert!(markers.iter().any(|marker| stderr.contains(marker)), "explicit --gpu must reach the renderer when the environment disables it: {stderr}");

    // Explicit flag overrides even bad environment value
    let out3 = d.join("out3.png");
    let o = bin().env("PHOTOCRAFT_GPU", "invalid").args(["convert", "--no-gpu", in_png.to_str().unwrap(), out3.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out3.is_file());

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn mcp_bridge_rejects_explicit_gpu_flags_and_ignores_env() {
    // Explicit --gpu with --bridge rejected
    let o = bin().args(["mcp", "--bridge", "127.0.0.1:50000", "--gpu"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("configure rendering on the desktop app"), "{stderr}");

    // Explicit --no-gpu with --bridge rejected
    let o = bin().args(["mcp", "--bridge", "127.0.0.1:50000", "--no-gpu"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("configure rendering on the desktop app"), "{stderr}");

    // An invalid PHOTOCRAFT_GPU value is ignored in bridge mode: the desktop process controls
    // rendering, so the bridge must not validate the variable or reject the request for it.
    let o = bin().env("PHOTOCRAFT_GPU", "invalid").args(["mcp", "--bridge", "127.0.0.1:50000"]).stdin(std::process::Stdio::null()).output().unwrap();
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!stderr.contains("PHOTOCRAFT_GPU"), "bridge mode must ignore the GPU environment: {stderr}");
}

#[test]
fn convert_and_run_honor_gpu_flag_and_preserve_stdout_protocol() {
    let d = tmp("protocol");
    let out_png = d.join("out.png");

    // run --gpu output on stdout is clean JSON
    let o =
        bin().args(["run", "--gpu", "--new", r#"{"width":32,"height":32}"#, "--cmd", "layer.new.layer", "--out", out_png.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out_png.is_file());
    let stdout = String::from_utf8_lossy(&o.stdout);
    for line in stdout.lines() {
        let v: Value = serde_json::from_str(line).expect("each stdout line must be valid JSON");
        assert!(v.get("command").is_some());
    }

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn batch_recovers_renderer_after_failed_item_and_preserves_gpu_policy() {
    let d = tmp("batch_regress");
    let in_dir = d.join("in");
    let out_dir = d.join("out");
    std::fs::create_dir_all(&in_dir).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();

    // First item is corrupt and fails to open
    let corrupt_png = in_dir.join("01_corrupt.png");
    std::fs::write(&corrupt_png, b"invalid png bytes").unwrap();

    // Second item is valid
    let valid_png = in_dir.join("02_valid.png");
    write_png(&valid_png, 32, 32);

    let actions = d.join("actions.json");
    std::fs::write(&actions, serde_json::json!([{"command": "layer.new.layer", "params": {"name": "Batch"}}]).to_string()).unwrap();
    let o = bin()
        .args(["batch", "--gpu", "--actions"])
        .arg(&actions)
        .args(["--in", in_dir.to_str().unwrap(), "--out", out_dir.to_str().unwrap()])
        .output()
        .unwrap();

    // Exit code 1 because 1 file failed
    assert_eq!(o.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&o.stdout);
    let stderr = String::from_utf8_lossy(&o.stderr);

    assert!(stdout.contains("1 succeeded, 1 failed"), "stdout: {stdout}");
    assert!(stderr.contains("FAIL"), "stderr must report failing item: {stderr}");
    let markers = ["GPU adapter:", "no GPU adapter available", "could not build GPU compositor", "falling back to CPU"];
    assert!(markers.iter().any(|marker| stderr.contains(marker)), "--gpu must reach the batch renderer: {stderr}");
    assert!(out_dir.join("02_valid.png").is_file(), "second item must have been saved successfully");

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn diagnostics_preserve_clean_stdout_and_emit_to_stderr() {
    let d = tmp("diag_clean");
    let in_png = d.join("in.png");
    let out_png = d.join("out.png");
    write_png(&in_png, 32, 32);

    let o = bin().args(["convert", "--gpu", in_png.to_str().unwrap(), out_png.to_str().unwrap()]).output().unwrap();

    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out_png.is_file());
    // Convert emits no stdout; diagnostics must go to stderr
    assert!(o.stdout.is_empty(), "convert stdout must remain completely empty, got: {:?}", String::from_utf8_lossy(&o.stdout));
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(stderr.contains("falling back to CPU"), "single-layer convert must report its CPU fallback on stderr: {stderr}");

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn serve_gpu_preserves_clean_protocol_stdout_and_drains_diagnostics_to_stderr() {
    use std::io::{BufRead, BufReader, Write};
    use std::process::Stdio;

    let mut child = bin().args(["serve", "--gpu"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // 1. Initial valid command
    writeln!(stdin, "{}", serde_json::json!({"id": 1, "method": "doc.new", "params": {"width": 16, "height": 16}})).unwrap();
    stdin.flush().unwrap();

    let mut line1 = String::new();
    assert!(stdout.read_line(&mut line1).unwrap() > 0, "server must respond");
    let v1: Value = serde_json::from_str(&line1).expect("line 1 on stdout must be valid JSON");
    assert_eq!(v1["id"], 1);
    assert_eq!(v1["ok"], true, "{v1}");

    // 2. A second layer so the render below composites instead of taking the native path.
    writeln!(stdin, "{}", serde_json::json!({"id": 2, "method": "engine.execute", "params": {"command": "layer.new.layer", "params": {"name": "Top"}}}))
        .unwrap();
    stdin.flush().unwrap();

    let mut line2 = String::new();
    assert!(stdout.read_line(&mut line2).unwrap() > 0, "server must respond to the layer command");
    let v2: Value = serde_json::from_str(&line2).expect("line 2 on stdout must be valid JSON");
    assert_eq!(v2["id"], 2);
    assert_eq!(v2["ok"], true, "{v2}");

    // 3. Failure recovery: invalid request receives error reply without crashing server
    writeln!(stdin, "{}", serde_json::json!({"id": 3, "method": "invalid.nonexistent.method", "params": {}})).unwrap();
    stdin.flush().unwrap();

    let mut line3 = String::new();
    assert!(stdout.read_line(&mut line3).unwrap() > 0, "server must respond to invalid request");
    let v3: Value = serde_json::from_str(&line3).expect("line 3 on stdout must be valid JSON");
    assert_eq!(v3["id"], 3);
    assert_eq!(v3["ok"], false, "{v3}");

    // 4. Subsequent valid command still succeeds (proves session & renderer recovery)
    writeln!(stdin, "{}", serde_json::json!({"id": 4, "method": "doc.inspect"})).unwrap();
    stdin.flush().unwrap();

    let mut line4 = String::new();
    assert!(stdout.read_line(&mut line4).unwrap() > 0, "server must respond to the inspect request");
    let v4: Value = serde_json::from_str(&line4).expect("line 4 on stdout must be valid JSON");
    assert_eq!(v4["id"], 4);
    assert_eq!(v4["ok"], true, "{v4}");

    // 5. Render the composite: one JSON line on stdout, diagnostics only on stderr.
    writeln!(stdin, "{}", serde_json::json!({"id": 5, "method": "doc.render", "params": {"maxSide": 16}})).unwrap();
    stdin.flush().unwrap();

    let mut line5 = String::new();
    assert!(stdout.read_line(&mut line5).unwrap() > 0, "server must answer the render request");
    let v5: Value = serde_json::from_str(&line5).expect("line 5 on stdout must be valid JSON");
    assert_eq!(v5["id"], 5);
    assert_eq!(v5["ok"], true, "{v5}");
    assert!(v5["result"]["base64"].as_str().is_some_and(|b| !b.is_empty()), "render must return PNG bytes: {v5}");

    // Close stdin and verify graceful exit
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());

    // Every render that reaches the renderer reports its adapter or why it fell back, whichever
    // build and adapter this test runs on.
    // `doc.render` returns the diagnostics of its own render in the reply's `warnings`; any left
    // in the queue go to stderr. Either way they stay off the protocol lines' framing.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reported = format!("{stderr}\n{}", v5["result"]["warnings"]);
    let markers = ["GPU adapter:", "no GPU adapter available", "could not build GPU compositor", "falling back to CPU", "GPU acceleration requested"];
    assert!(markers.iter().any(|m| reported.contains(m)), "render must report its GPU backend or fallback in its reply or on stderr: {reported}");
}

#[test]
fn cpu_default_does_not_emit_gpu_warnings() {
    let d = tmp("cpu_default");
    let in_png = d.join("in.png");
    let out_png = d.join("out.png");
    write_png(&in_png, 16, 16);

    let o = bin().args(["convert", in_png.to_str().unwrap(), out_png.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out_png.is_file());

    let stderr = String::from_utf8_lossy(&o.stderr);
    assert!(!stderr.contains("GPU"), "default CPU convert should not emit GPU warnings, got: {stderr}");

    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn run_gpu_quick_export_reports_that_it_runs_on_the_cpu() {
    let d = tmp("quick_export");
    let out_png = d.join("quick.png");
    let params = format!(r#"{{"path":{}}}"#, serde_json::to_string(&out_png.to_string_lossy()).unwrap());
    let args = ["run", "--new", r#"{"width":16,"height":16,"background":"white"}"#, "--cmd", "file.export.quickExport", "--params", params.as_str()];
    let o = bin().args(args).arg("--gpu").output().unwrap();
    assert!(o.status.success(), "stderr: {}", String::from_utf8_lossy(&o.stderr));
    assert!(out_png.is_file());
    let stderr = String::from_utf8_lossy(&o.stderr);
    assert_eq!(stderr.matches("quickExport runs on the CPU compositor").count(), 1, "one CPU note expected: {stderr}");

    let o = bin().args(args).output().unwrap();
    assert!(o.status.success());
    assert!(!String::from_utf8_lossy(&o.stderr).contains("runs on the CPU compositor"), "no note without --gpu");
    let _ = std::fs::remove_dir_all(&d);
}
