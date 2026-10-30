//! Process-level tests of the `photocraft-cli` binary.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// The binary, without the variables `mcp` and `serve` fall back to: a test that wants one sets it.
fn bin() -> Command {
    let mut cmd = Command::new(crate::binary());
    for var in ENV_VARS {
        cmd.env_remove(var);
    }
    cmd
}

const ENV_VARS: [&str; 3] = ["PHOTOCRAFT_CONTROL_PORT", "PHOTOCRAFT_CONTROL_TOKEN", "PHOTOCRAFT_CONTROL_TOKEN_FILE"];

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pc-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_png(path: &Path, w: u32, h: u32, seed: u8) {
    let data: Vec<u8> = (0..w * h * 4).map(|i| (i as u8).wrapping_mul(seed).wrapping_add(seed)).collect();
    let img = photocraft_codecs::Image::from_u8(w, h, photocraft_codecs::ChannelLayout::Rgba, data).unwrap();
    std::fs::write(path, photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
}

/// What a successful run printed.
struct Printed {
    stdout: String,
    stderr: String,
}

fn ok(cmd: &mut Command) -> Printed {
    let o = cmd.output().unwrap();
    let (stdout, stderr) = (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned());
    assert!(o.status.success(), "exit {:?}\nstdout: {stdout}\nstderr: {stderr}", o.status);
    Printed { stdout, stderr }
}

#[test]
fn usage_and_unknown_command() {
    let o = bin().output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("convert"));
    let o = bin().arg("frobnicate").output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    // `help` names a subcommand or nothing: anything else is a usage error too.
    let o = bin().args(["help", "frobnicate"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(!o.stderr.is_empty());
    let out = ok(bin().arg("--help")).stdout;
    assert!(out.contains("convert"));
    let out = ok(bin().arg("--version")).stdout;
    assert!(out.starts_with("photocraft-cli "));
}

/// #423: `<subcommand> --help` and `-h` print usage and return at once (no inputs read, no files
/// written, no server started), whatever else is on the line.
#[test]
fn every_subcommand_answers_help() {
    let d = tmp("help");
    for sub in ["convert", "info", "run", "batch", "droplet", "commands", "mcp", "serve"] {
        for help in ["--help", "-h"] {
            let o = bin().current_dir(&d).arg(sub).arg(help).arg("--out").arg("x.png").stdin(Stdio::null()).output().unwrap();
            let out = String::from_utf8_lossy(&o.stdout);
            assert_eq!(o.status.code(), Some(0), "{sub} {help}: {}", String::from_utf8_lossy(&o.stderr));
            assert!(out.contains(&format!("photocraft-cli {sub}")), "{sub} {help}: {out}");
        }
    }
    // Help where a flag's value should be is still help, not a missing value or a file named `-h`.
    // So is help after another mistake.
    for args in [
        &["run", "x.png", "--out", "--help"][..],
        &["commands", "--filter", "-h"],
        &["mcp", "--bridge", "-h"],
        &["serve", "--port", "--help"],
        &["mcp", "stray.psd", "--help"],
        &["serve", "--port", "abc", "--port", "1", "-h"],
    ] {
        let o = bin().current_dir(&d).args(args).stdin(Stdio::null()).output().unwrap();
        assert_eq!(o.status.code(), Some(0), "{args:?}: {}", String::from_utf8_lossy(&o.stderr));
        assert!(String::from_utf8_lossy(&o.stdout).contains(&format!("photocraft-cli {}", args[0])), "{args:?}");
    }
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0, "help wrote nothing");
}

/// #423: a flag the subcommand doesn't take is a usage error naming it, before any work.
#[test]
fn unknown_flags_are_usage_errors() {
    let d = tmp("flags");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    write_png(&input.join("a.png"), 8, 4, 3);
    let actions = d.join("actions.json");
    std::fs::write(&actions, r#"[{"command":"image.adjustments.invert"}]"#).unwrap();
    let out_dir = d.join("out");
    let fails = |cmd: &mut Command, flag: &str| {
        let o = cmd.output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "{err}");
        assert!(err.contains(flag), "{err}");
    };
    for typo in [&["--fromat", "jpg"][..], &["--fromat=jpg"][..]] {
        fails(bin().args(["batch", "--actions"]).arg(&actions).arg("--in").arg(&input).arg("--out").arg(&out_dir).args(typo), "--fromat");
        assert!(!out_dir.exists(), "nothing written");
    }
    let x = d.join("x.png");
    fails(bin().arg("run").arg(input.join("a.png")).args(["--cmd", "image.adjustments.invert", "--param", "{}", "--out"]).arg(&x), "--param");
    assert!(!x.exists());
    // A flag one subcommand takes is still unknown to another; a bare flag takes no value.
    fails(bin().args(["info", "--json"]).arg(input.join("a.png")), "--json");
    let o = bin().args(["commands", "--json=yes"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--json"));
    // A flag can't be a value: a missing value is reported, not filled with the next flag.
    let o = bin()
        .args(["batch", "--actions"])
        .arg(&actions)
        .arg("--in")
        .arg(&input)
        .arg("--out")
        .arg(&out_dir)
        .args(["--format", "--quality", "80"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("a value is required for '--format"));
    assert!(!out_dir.exists(), "nothing written");
    // `--flag=value` still passes a value that starts with `--`.
    ok(bin().args(["commands", "--filter=--json"]));
    // Every flag still works in both forms.
    let out = ok(bin().args(["commands", "--json", "--filter=gaussian"])).stdout;
    assert!(out.contains("filter.blur.gaussianBlur"), "{out}");
    // The Elliptical Marquee is a `select.rect` option, found by its keywords (#5060).
    for needle in ["ellipse", "marquee"] {
        let out = ok(bin().args(["commands", "--filter", needle])).stdout;
        assert!(out.contains("select.rect"), "{needle}: {out}");
    }
    ok(bin().args(["batch", "--actions"]).arg(&actions).arg(format!("--in={}", input.display())).arg("--out").arg(&out_dir).args([
        "--format=jpg",
        "--quality",
        "80",
    ]));
    assert!(out_dir.join("a.jpg").exists());
    let out = ok(bin().arg("info").arg(input.join("a.png")).arg("--compact")).stdout;
    assert!(!out.trim().contains('\n'), "compact JSON is one line");
}

fn server_usage_error(args: &[&str], expected: &str) {
    let o = bin().args(args).env_remove("PHOTOCRAFT_CONTROL_TOKEN").env_remove("PHOTOCRAFT_CONTROL_TOKEN_FILE").stdin(Stdio::null()).output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{args:?}: {err}");
    assert!(o.stdout.is_empty(), "usage errors must leave protocol stdout empty: {args:?}");
    assert!(err.contains(expected), "{args:?}: expected {expected:?} in {err}");
}

#[test]
fn server_modes_reject_ignored_or_conflicting_options() {
    for sub in ["mcp", "serve"] {
        server_usage_error(&[sub, "stray.psd"], "unexpected argument 'stray.psd'");
        for flag in ["--control-token", "--control-token-file"] {
            server_usage_error(&[sub, flag, "unused"], if sub == "mcp" { "requires --bridge" } else { "requires --port" });
        }
        for flag in ["--automation-read-root", "--automation-write-root"] {
            server_usage_error(&[sub, flag, ".", flag, "."], "cannot be used multiple times");
            server_usage_error(&[sub, &format!("{flag}=")], &format!("a value is required for '{flag} "));
            server_usage_error(&[sub, flag, "--automation-write-root", "."], &format!("a value is required for '{flag} "));
        }
        let transport = if sub == "mcp" { "--bridge" } else { "--port" };
        for flag in [transport, "--control-token", "--control-token-file"] {
            // Valid values, so the repeat is what is refused (a transport value is parsed first).
            server_usage_error(&[sub, flag, "7878", &format!("{flag}=7879")], "cannot be used multiple times");
            server_usage_error(&[sub, flag, ""], &format!("for '{flag} "));
        }
        server_usage_error(&[sub, transport, "7878", "--control-token", "unused", "--control-token-file", "unused"], "cannot be used with");
    }
    for flag in ["--automation-read-root", "--automation-write-root"] {
        server_usage_error(&["mcp", "--bridge", "127.0.0.1:7878", flag, "."], "configure automation roots on the desktop app");
    }
}

/// Each launch line would, before validation, have started a TCP server and written the token file.
#[test]
fn server_validation_does_not_consume_stdio_or_create_files() {
    let d = tmp("server-validation");
    let token = d.join("must-not-be-created.token");
    let other = d.join("other.token");
    let token_flag = format!("--control-token-file={}", token.display());
    let other_flag = format!("--control-token-file={}", other.display());
    for tail in [&["stray"][..], &["--automation-read-root", ".", "--automation-read-root", "."], &["--port", "0"], &[&other_flag]] {
        let mut child = bin()
            .current_dir(&d)
            .args(["serve", "--port", "0", &token_flag])
            .args(tail)
            .env_remove("PHOTOCRAFT_CONTROL_TOKEN")
            .env_remove("PHOTOCRAFT_CONTROL_TOKEN_FILE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Keep stdin open: invalid launch arguments must fail before starting a server.
        let input = child.stdin.take().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if child.try_wait().unwrap().is_none() {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("{tail:?}: invalid configuration started a server");
        }
        drop(input);
        let o = child.wait_with_output().unwrap();
        assert_eq!(o.status.code(), Some(2), "{tail:?}: {}", String::from_utf8_lossy(&o.stderr));
        assert!(o.stdout.is_empty(), "{tail:?}");
        assert!(!token.exists() && !other.exists(), "{tail:?}: validation must happen before creating credentials");
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn convert_png_pcraft_png_is_lossless() {
    let d = tmp("convert");
    let (a, b, c) = (d.join("a.png"), d.join("b.pcraft"), d.join("c.png"));
    write_png(&a, 37, 23, 7);
    ok(bin().args(["convert"]).arg(&a).arg(&b));
    assert!(photocraft_format::is_pcraft(&std::fs::read(&b).unwrap()));
    ok(bin().args(["convert"]).arg(&b).arg(&c));
    let x = photocraft_codecs::decode(&std::fs::read(&a).unwrap()).unwrap();
    let y = photocraft_codecs::decode(&std::fs::read(&c).unwrap()).unwrap();
    assert_eq!(x.to_rgba8(), y.to_rgba8());
    // Format override and lossy warning path.
    ok(bin().args(["convert"]).arg(&a).arg(d.join("out.bin")).args(["--format", "jpg", "--quality", "60"]));
    assert!(photocraft_codecs::detect(&std::fs::read(d.join("out.bin")).unwrap()) == Some(photocraft_codecs::Format::Jpeg));
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn convert_errors() {
    let d = tmp("converr");
    let o = bin().args(["convert", "/missing/in.png"]).arg(d.join("x.png")).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let o = bin().args(["convert", "only-one"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn info_prints_layer_tree_json() {
    let d = tmp("info");
    let a = d.join("a.png");
    write_png(&a, 10, 6, 3);
    let out = ok(bin().arg("info").arg(&a)).stdout;
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["width"], 10);
    assert_eq!(v["height"], 6);
    // An RGBA PNG opens as a normal "Layer 0", not the locked Background (as in Photoshop).
    assert_eq!(v["layers"][0]["name"], "Layer 0");
    assert!(v.get("history").is_none());
    let out = ok(bin().arg("info").arg(&a).arg("--compact")).stdout;
    assert_eq!(out.trim().lines().count(), 1);
    std::fs::remove_dir_all(d).unwrap();
}

/// #518, #523: a truncated JPEG and an animated GIF open, but `info` lists why the image is
/// incomplete and `convert` prints it on stderr (and still succeeds); complete files stay quiet.
#[test]
fn info_and_convert_report_decode_warnings() {
    let d = tmp("decode-warnings");
    let noise: Vec<u8> = (0..64 * 48 * 3).map(|i: u32| (i.wrapping_mul(7919) % 251) as u8).collect();
    let img = photocraft_codecs::Image::from_u8(64, 48, photocraft_codecs::ChannelLayout::Rgb, noise).unwrap();
    let jpeg = photocraft_codecs::encode(&img, photocraft_codecs::Format::Jpeg, &Default::default()).unwrap();
    // Three 1x1 frames.
    let mut gif = b"GIF89a\x01\0\x01\0\x80\0\0\0\0\0\xFF\xFF\xFF".to_vec();
    for _ in 0..3 {
        gif.extend_from_slice(b"\x2C\0\0\0\0\x01\0\x01\0\0\x02\x02\x44\x01\0");
    }
    gif.push(0x3B);
    // A GIF opens in Indexed Color (#7886), so `convert` to PNG also notes the palette PNG it
    // writes, after the decode warning.
    let cases = [
        ("whole.jpg", &jpeg[..], None, None),
        ("half.jpg", &jpeg[..jpeg.len() / 2], Some("JPEG data ends early (the file is truncated or damaged); part of the image is missing"), None),
        ("anim.gif", &gif[..], Some("only the first of 3 frames was imported"), Some("written as an 8-bit palette PNG (2 colours)")),
    ];
    for (name, bytes, warning, export) in cases {
        let file = d.join(name);
        std::fs::write(&file, bytes).unwrap();
        let out = ok(bin().arg("info").arg(&file).arg("--compact")).stdout;
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["warnings"], json!(warning.into_iter().collect::<Vec<_>>()), "{name}");
        let err = ok(bin().arg("convert").arg(&file).arg(d.join(format!("{name}.png")))).stderr;
        let expected: Vec<String> = warning.into_iter().chain(export).map(|w| format!("warning: {w}")).collect();
        assert_eq!(
            err.trim(),
            expected.join(
                "
"
            ),
            "{name}"
        );
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn run_commands_and_save() {
    let d = tmp("run");
    let out_pc = d.join("r.pcraft");
    let out = ok(bin()
        .args(["run", "--new", r#"{"width":40,"height":30,"name":"Run"}"#])
        .args(["--cmd", "layer.new.layer", "--params", r#"{"name":"Ink"}"#])
        .args(["--cmd", "paint.stroke", "--params", r##"{"points":[[2,2,1],[30,20,1]],"size":5,"color":"#00ff00"}"##])
        .args(["--cmd", "document.inspect"])
        .arg("--out")
        .arg(&out_pc))
    .stdout;
    let lines: Vec<Value> = out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[2]["command"], "document.inspect");
    assert_eq!(lines[2]["result"]["name"], "Run");
    let info = ok(bin().arg("info").arg(&out_pc)).stdout;
    let v: Value = serde_json::from_str(&info).unwrap();
    // Saving a native document gives the reopened bundle its output filename.
    assert_eq!(v["name"], "r.pcraft");
    let names: Vec<&str> = v["layers"].as_array().unwrap().iter().filter_map(|l| l["name"].as_str()).collect();
    assert_eq!(names, ["Ink", "Background"]);
    // Continue from the saved file and export PNG.
    let png = d.join("r.png");
    ok(bin().arg("run").arg(&out_pc).args(["--cmd", "layer.new.layer"]).arg("--out").arg(&png));
    assert_eq!(photocraft_codecs::decode(&std::fs::read(&png).unwrap()).unwrap().dimensions(), (40, 30));
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn run_stops_after_failed_action_without_exporting() {
    let d = tmp("run-failed-action");
    let out = d.join("result.png");
    let result = bin()
        .args(["run", "--new", r#"{"width":8,"height":8}"#])
        .args(["--cmd", "layer.new.layer"])
        .args(["--cmd", "actions.record", "--params", r#"{"name":"Delete"}"#])
        .args(["--cmd", "layer.delete", "--cmd", "actions.stop"])
        // The recorded deletion succeeded with two layers; replay cannot delete the last one.
        .args(["--cmd", "actions.play", "--params", r#"{"action":"Delete"}"#])
        .args(["--cmd", "document.inspect", "--out"])
        .arg(&out)
        .output()
        .unwrap();
    let error = String::from_utf8_lossy(&result.stderr);
    assert_eq!(result.status.code(), Some(1), "stdout: {}\nstderr: {error}", String::from_utf8_lossy(&result.stdout));
    assert!(error.contains("actions.play") && error.contains("Delete") && error.contains("step 1"), "{error}");
    let rows: Vec<Value> = String::from_utf8_lossy(&result.stdout).lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    assert!(rows.iter().all(|row| row["command"] != "document.inspect"), "later commands must not run");
    assert!(!out.exists(), "a failed action must not export a partial result");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn run_continues_after_successful_action_and_exports() {
    let d = tmp("run-successful-action");
    let out = d.join("result.png");
    let stdout = ok(bin()
        .args(["run", "--new", r#"{"width":8,"height":8}"#])
        .args(["--cmd", "actions.record", "--params", r#"{"name":"Add"}"#])
        .args(["--cmd", "layer.new.layer", "--cmd", "actions.stop"])
        .args(["--cmd", "actions.play", "--params", r#"{"action":"Add"}"#])
        .args(["--cmd", "document.inspect", "--out"])
        .arg(&out))
    .stdout;
    let rows: Vec<Value> = stdout.lines().map(|line| serde_json::from_str(line).unwrap()).collect();
    let playback = rows.iter().find(|row| row["command"] == "actions.play").unwrap();
    assert_eq!(playback["result"]["ran"], 1);
    assert!(playback["result"].get("failed").is_none());
    assert!(rows.iter().any(|row| row["command"] == "document.inspect"));
    assert!(out.is_file());
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn run_errors() {
    let o = bin().args(["run", "--new", "{}", "--cmd", "no.such"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown command"));
    // Neither a --cmd nor --out, and JSON that doesn't parse, are usage errors naming the flag.
    let o = bin().args(["run", "--new", "{}", "--params", "{}"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--cmd"));
    let o = bin().args(["run", "--new", "{", "--cmd", "x"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&o.stderr).contains("--new"));
    // Params must be a JSON object, for `--new` and for each `--params`.
    let o = bin().args(["run", "--new", "[3]", "--cmd", "layer.new.layer"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("--new") && e.contains("must be a JSON object"), "{e}");
    let o = bin().args(["run", "--new", "{}", "--cmd", "layer.new.layer", "--params", "\"x\""]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("layer.new.layer") && e.contains("must be a JSON object"), "{e}");
}

#[test]
fn run_rejects_invalid_filter_parameters_without_exporting() {
    let d = tmp("invalid-filter-params");
    let out = d.join("result.png");
    for (params, key) in [(r#"{"radius":"big"}"#, "radius"), (r#"{"radus":3}"#, "radus"), (r#"{"radius":5000}"#, "radius"), (r#"{"radius":-4}"#, "radius")] {
        let result = bin()
            .args(["run", "--new", r#"{"width":8,"height":8}"#, "--cmd", "filter.blur.gaussianBlur", "--params", params, "--out"])
            .arg(&out)
            .output()
            .unwrap();
        assert_eq!(result.status.code(), Some(1));
        let error = String::from_utf8_lossy(&result.stderr);
        assert!(error.contains("filter.blur.gaussianBlur") && error.contains(key), "{error}");
        assert!(!out.exists());
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn batch_applies_actions_to_directory() {
    let d = tmp("batch");
    let (inp, outp) = (d.join("in"), d.join("out"));
    std::fs::create_dir_all(&inp).unwrap();
    write_png(&inp.join("one.png"), 12, 8, 1);
    write_png(&inp.join("two.png"), 9, 9, 2);
    std::fs::write(inp.join("notes.txt"), "ignored").unwrap();
    let actions = d.join("actions.json");
    std::fs::write(&actions, json!([{"command": "layer.new.layer", "params": {"name": "Batch"}}]).to_string()).unwrap();
    let out = ok(bin().args(["batch", "--actions"]).arg(&actions).arg("--in").arg(&inp).arg("--out").arg(&outp).args(["--format", "pcraft"])).stdout;
    assert!(out.contains("2 succeeded, 0 failed"), "{out}");
    for n in ["one", "two"] {
        let info = ok(bin().arg("info").arg(outp.join(format!("{n}.pcraft")))).stdout;
        let v: Value = serde_json::from_str(&info).unwrap();
        assert_eq!(v["layers"][0]["name"], "Batch");
    }
    // A failing action reports and exits 1.
    std::fs::write(&actions, r#"{"actions":[{"id":"no.such.command"}]}"#).unwrap();
    let o = bin().args(["batch", "--actions"]).arg(&actions).arg("--in").arg(&inp).arg("--out").arg(&outp).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("FAIL"));
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn batch_skips_recorded_view_and_save_steps() {
    let d = tmp("batch-shell-steps");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    let source = input.join("a.png");
    write_png(&source, 12, 8, 3);
    let original = std::fs::read(&source).unwrap();
    let recorded_save = d.join("recorded-save.psd");
    let mut cases = Vec::new();
    for (name, id) in [("zoom-in", "view.zoomIn"), ("zoom-out", "view.zoomOut"), ("fit", "view.fitOnScreen"), ("actual-pixels", "view.actualPixels")] {
        cases.push((name, json!([[id, {}], ["image.imageRotation.90cw", {}]]).to_string()));
    }
    cases.push(("save", json!([["file.save", {}], ["image.imageRotation.90cw", {}]]).to_string()));
    cases.push(("save-as", json!([["file.saveAs", {"path": recorded_save.to_string_lossy()}], ["image.imageRotation.90cw", {}]]).to_string()));

    for (name, actions) in cases {
        let o = batch_with(&d, &input, name, &actions, &[]);
        let (stdout, stderr) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        assert!(o.status.success(), "{name}: {stdout}\n{stderr}");
        assert!(stdout.contains("1 succeeded, 0 failed"), "{name}: {stdout}");
        let output = d.join(name).join("a.png");
        let info = ok(bin().arg("info").arg(&output)).stdout;
        let info: Value = serde_json::from_str(&info).unwrap();
        assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(8), Some(12)), "{name}: {info}");
        assert_eq!(std::fs::read(&source).unwrap(), original, "{name} changed its input");
        assert!(!recorded_save.exists(), "{name} wrote the recorded Save As path");
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn parse_actions_formats() {
    let a = photocraft_cli::parse_actions(r#"[{"command":"a","params":{"x":1}},{"id":"b"}]"#).unwrap();
    assert_eq!(a, vec![("a".to_string(), json!({"x":1})), ("b".to_string(), json!({}))]);
    assert!(photocraft_cli::parse_actions(r#"{"actions":[{"command":"c"}]}"#).unwrap().len() == 1);
    assert!(photocraft_cli::parse_actions(r#"[{"params":{}}]"#).is_err());
    assert!(photocraft_cli::parse_actions("7").is_err());
    // #489: the step shapes a recorded action and a droplet store, bare or wrapped.
    let want = vec![("a".to_string(), json!({"x":1})), ("b".to_string(), json!({}))];
    for text in [
        r#"[["a",{"x":1}],["b"]]"#,
        r#"[["a",{"x":1}],"b"]"#,
        r#"{"steps":[["a",{"x":1}],["b",{}]]}"#,
        r#"{"photocraftDroplet":1,"action":{"steps":[["a",{"x":1}],{"command":"b"}]}}"#,
        r#"{"actions":[["a",{"x":1}],["b",{}]]}"#,
    ] {
        assert_eq!(photocraft_cli::parse_actions(text).unwrap(), want, "{text}");
    }
    for text in ["[42]", "[[]]", "[[7,{}]]", r#"{"name":"x"}"#] {
        assert!(photocraft_cli::parse_actions(text).is_err(), "{text}");
    }
}

/// Runs `batch` on `input` with `actions` (saved as `<name>.json`), writing to the folder `<d>/<name>`.
fn batch_with(d: &Path, input: &Path, name: &str, actions: &str, extra: &[&str]) -> std::process::Output {
    let file = d.join(format!("{name}.json"));
    std::fs::write(&file, actions).unwrap();
    bin().args(["batch", "--actions"]).arg(&file).arg("--in").arg(input).arg("--out").arg(d.join(name)).args(extra).output().unwrap()
}

/// #489: `batch --actions` takes `[id, params]` steps and a droplet's steps, with the same results
/// as the object form; a malformed step fails before anything is written.
#[test]
fn batch_accepts_recorded_action_steps() {
    let d = tmp("batch-steps");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    write_png(&input.join("a.png"), 8, 4, 3);
    write_png(&input.join("b.png"), 6, 5, 9);
    let shapes = [
        ("objects", r#"[{"command":"image.adjustments.invert"}]"#),
        ("pairs", r#"[["image.adjustments.invert",{}]]"#),
        ("ids", r#"["image.adjustments.invert"]"#),
        ("droplet", r#"{"photocraftDroplet":1,"action":{"steps":[["image.adjustments.invert",{}]]}}"#),
    ];
    for (name, actions) in shapes {
        let o = batch_with(&d, &input, name, actions, &[]);
        assert!(o.status.success(), "{name}: {}", String::from_utf8_lossy(&o.stderr));
        assert!(String::from_utf8_lossy(&o.stdout).contains("2 succeeded, 0 failed"), "{name}");
        for f in ["a.png", "b.png"] {
            assert_eq!(std::fs::read(d.join(name).join(f)).unwrap(), std::fs::read(d.join("objects").join(f)).unwrap(), "{name}/{f}");
        }
    }
    let o = batch_with(&d, &input, "bad", "[42]", &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!d.join("bad").exists(), "nothing written");
}

/// #490: a leading dot on `--format` doesn't double the dot in output names.
#[test]
fn batch_format_with_leading_dot() {
    let d = tmp("batch-dot");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    write_png(&input.join("a.png"), 8, 4, 3);
    write_png(&input.join("b.png"), 6, 5, 9);
    let invert = r#"[{"command":"image.adjustments.invert"}]"#;
    for (name, format) in [("dot", ".jpg"), ("plain", "jpg")] {
        let o = batch_with(&d, &input, name, invert, &["--format", format]);
        assert!(o.status.success(), "{format}: {}", String::from_utf8_lossy(&o.stderr));
        let mut names: Vec<String> = std::fs::read_dir(d.join(name)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["a.jpg", "b.jpg"], "{format}");
        assert_eq!(photocraft_codecs::detect(&std::fs::read(d.join(name).join("a.jpg")).unwrap()), Some(photocraft_codecs::Format::Jpeg));
    }
}

/// #491: `--quality` outside 1-100 is a usage error for every subcommand, whatever the number's
/// size, and nothing is written.
#[test]
fn quality_outside_1_to_100_is_refused() {
    let d = tmp("quality");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    let a = input.join("a.png");
    write_png(&a, 8, 4, 3);
    let invert = r#"[{"command":"image.adjustments.invert"}]"#;
    for q in ["0", "101", "255", "256", "1000", "-1", "abc", "50.5", ""] {
        let out = d.join("q.jpg");
        let (mut convert, mut run) = (bin(), bin());
        convert.arg("convert").arg(&a).arg(&out).args(["--quality", q]);
        run.arg("run").arg(&a).args(["--cmd", "image.adjustments.invert", "--out"]).arg(&out).args(["--quality", q]);
        for cmd in [&mut convert, &mut run] {
            let o = cmd.output().unwrap();
            assert_eq!(o.status.code(), Some(2), "--quality {q}");
            assert!(String::from_utf8_lossy(&o.stderr).contains("--quality"), "--quality {q}");
            assert!(!out.exists(), "--quality {q} wrote a file");
        }
        let o = batch_with(&d, &input, "qb", invert, &["--format", "jpg", "--quality", q]);
        assert_eq!(o.status.code(), Some(2), "batch --quality {q}");
        assert!(!d.join("qb").exists(), "batch --quality {q} wrote a folder");
    }
    // The documented range still encodes, lower quality making a smaller file.
    let sizes: Vec<usize> = ["1", "50", "100"]
        .iter()
        .map(|q| {
            let out = d.join(format!("q{q}.jpg"));
            ok(bin().arg("convert").arg(&a).arg(&out).args(["--quality", q]));
            std::fs::read(out).unwrap().len()
        })
        .collect();
    assert!(sizes[0] <= sizes[1] && sizes[1] <= sizes[2], "{sizes:?}");
}

/// DPX converts both ways, `info` reports its size, mode and depth, and `--dpx-bits` picks the
/// bits per sample (byte 803 of the header); anything but 8, 10, 12 or 16 is a usage error (#1049).
#[test]
fn dpx_converts_both_ways_with_the_bits_asked_for() {
    let d = tmp("dpx");
    let a = d.join("a.png");
    write_png(&a, 8, 4, 3);
    for (bits, want) in [(None, 8), (Some("10"), 10), (Some("12"), 12), (Some("16"), 16)] {
        let out = d.join("a.dpx");
        let mut convert = bin();
        convert.arg("convert").arg(&a).arg(&out);
        if let Some(b) = bits {
            convert.args(["--dpx-bits", b]);
        }
        ok(&mut convert);
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!((&bytes[..4], bytes[803]), (&b"SDPX"[..], want), "--dpx-bits {bits:?}");
    }
    let info = ok(bin().arg("info").arg(d.join("a.dpx")).arg("--compact")).stdout;
    let v: Value = serde_json::from_str(&info).unwrap();
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(8), Some(4)), "{v}");
    assert!(info.contains("16") && info.to_lowercase().contains("rgb"), "{info}");
    let back = d.join("back.png");
    ok(bin().arg("convert").arg(d.join("a.dpx")).arg(&back));
    assert!(back.exists());
    for bad in ["24", "0", "abc"] {
        let out = d.join("bad.dpx");
        let o = bin().arg("convert").arg(&a).arg(&out).args(["--dpx-bits", bad]).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "--dpx-bits {bad}");
        assert!(err.contains("--dpx-bits") && err.contains("expected 8, 10, 12 or 16"), "--dpx-bits {bad}: {err}");
        assert!(!out.exists(), "--dpx-bits {bad} wrote a file");
    }
}

/// DDS converts both ways; `--dds-compression` picks the FourCC (bytes 84–87; none writes
/// uncompressed pixels) and `--dds-mipmaps` the mip count (bytes 28–31). A compression that
/// isn't none, bc1, bc2 or bc3 is a usage error and nothing is written (#1327).
#[test]
fn dds_converts_both_ways_with_the_compression_asked_for() {
    let d = tmp("dds");
    let a = d.join("a.png");
    write_png(&a, 8, 4, 3);
    for (args, fourcc, mips) in [
        (&[][..], [0; 4], 1),
        (&["--dds-compression", "bc1"][..], *b"DXT1", 1),
        (&["--dds-compression", "DXT3", "--dds-mipmaps"][..], *b"DXT3", 4),
        (&["--dds-compression", "bc3"][..], *b"DXT5", 1),
        (&["--dds-mipmaps"][..], [0; 4], 4),
    ] {
        let out = d.join("a.dds");
        ok(bin().arg("convert").arg(&a).arg(&out).args(args));
        let bytes = std::fs::read(&out).unwrap();
        let mip_count = u32::from_le_bytes(bytes[28..32].try_into().unwrap());
        assert_eq!((&bytes[..4], &bytes[84..88], mip_count), (&b"DDS "[..], &fourcc[..], mips), "{args:?}");
        let back = d.join("back.png");
        ok(bin().arg("convert").arg(&out).arg(&back));
        assert!(back.exists(), "{args:?}");
    }
    for bad in ["bc7", "dxt2", ""] {
        let out = d.join("bad.dds");
        let o = bin().arg("convert").arg(&a).arg(&out).args(["--dds-compression", bad]).output().unwrap();
        assert_eq!(o.status.code(), Some(2), "--dds-compression {bad}");
        assert!(String::from_utf8_lossy(&o.stderr).contains("--dds-compression"), "--dds-compression {bad}");
        assert!(!out.exists(), "--dds-compression {bad} wrote a file");
    }
}

/// `--tga-bits` picks a TGA's bits per pixel (byte 16 of its header); anything but 24 or 32 is a
/// usage error and nothing is written.
#[test]
fn tga_bits_choose_the_targa_depth() {
    let d = tmp("tga-bits");
    let a = d.join("a.png");
    write_png(&a, 8, 4, 3);
    for (bits, want) in [(None, 32), (Some("24"), 24), (Some("32"), 32)] {
        let out = d.join("a.tga");
        let mut convert = bin();
        convert.arg("convert").arg(&a).arg(&out);
        if let Some(b) = bits {
            convert.args(["--tga-bits", b]);
        }
        ok(&mut convert);
        assert_eq!(std::fs::read(&out).unwrap()[16], want, "--tga-bits {bits:?}");
    }
    for bad in ["16", "0", "abc", ""] {
        let out = d.join("bad.tga");
        let o = bin().arg("convert").arg(&a).arg(&out).args(["--tga-bits", bad]).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "--tga-bits {bad}");
        assert!(err.contains("--tga-bits") && err.contains("expected 24 or 32"), "--tga-bits {bad}: {err}");
        assert!(!out.exists(), "--tga-bits {bad} wrote a file");
    }
}

/// #7437: `run <file> --out` to that same file, however `--out` is spelt, is refused before the
/// file is read or any command runs, unless `--in-place` asks for it (as `batch` does, #492).
#[test]
fn run_refuses_to_write_over_its_input() {
    let d = tmp("run-in-place");
    write_png(&d.join("in.png"), 8, 4, 7);
    let before = std::fs::read(d.join("in.png")).unwrap();
    let run = |out: &Path, extra: &[&str]| {
        bin().current_dir(&d).args(["run", "in.png", "--cmd", "image.adjustments.invert", "--out"]).arg(out).args(extra).output().unwrap()
    };
    let mut outs = vec![PathBuf::from("in.png"), PathBuf::from("./in.png"), d.join("in.png"), d.join(".").join("in.png")];
    std::fs::hard_link(d.join("in.png"), d.join("hard.png")).unwrap();
    outs.push(PathBuf::from("hard.png"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(d.join("in.png"), d.join("soft.png")).unwrap();
        outs.push(PathBuf::from("soft.png"));
    }
    for out in &outs {
        for extra in [&[][..], &["--format", "png"][..]] {
            let o = run(out, extra);
            let (stdout, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
            assert_eq!(o.status.code(), Some(1), "--out {} {extra:?}: {err}", out.display());
            assert!(err.contains("is the input file") && err.contains("--in-place"), "{err}");
            assert!(stdout.is_empty(), "--out {} {extra:?} ran commands: {stdout}", out.display());
            assert_eq!(std::fs::read(d.join("in.png")).unwrap(), before, "--out {} {extra:?} changed the original", out.display());
        }
    }
    // Another file is written as before, leaving the original alone.
    ok(bin().current_dir(&d).args(["run", "in.png", "--cmd", "image.adjustments.invert", "--out", "out.png"]));
    assert!(d.join("out.png").is_file());
    assert_eq!(std::fs::read(d.join("in.png")).unwrap(), before);
    // `--in-place` opts in: the inverted result replaces the original, which still opens.
    ok(bin().current_dir(&d).args(["run", "in.png", "--cmd", "image.adjustments.invert", "--out", "in.png", "--in-place"]));
    assert_ne!(std::fs::read(d.join("in.png")).unwrap(), before, "--in-place left the original unchanged");
    let out = ok(bin().current_dir(&d).args(["info", "in.png", "--compact"])).stdout;
    let info: Value = serde_json::from_str(&out).unwrap();
    assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(8), Some(4)), "{info}");
}

/// `--jpeg-format` picks the JPEG's scans: progressive writes a progressive frame (SOF2). One it
/// doesn't name is a usage error and nothing is written.
#[test]
fn jpeg_format_chooses_the_scans() {
    let d = tmp("jpeg-format");
    let a = d.join("a.png");
    write_png(&a, 16, 8, 3);
    let marker = |b: &[u8], m: u8| b.windows(2).any(|w| w == [0xFF, m]);
    for (format, progressive) in [(None, false), (Some("baseline"), false), (Some("optimized"), false), (Some("progressive"), true)] {
        let out = d.join("a.jpg");
        let mut convert = bin();
        convert.arg("convert").arg(&a).arg(&out);
        if let Some(f) = format {
            convert.args(["--jpeg-format", f]);
        }
        ok(&mut convert);
        assert_eq!(marker(&std::fs::read(&out).unwrap(), 0xC2), progressive, "--jpeg-format {format:?}");
    }
    for bad in ["Progressive", "1", ""] {
        let out = d.join("bad.jpg");
        let o = bin().arg("convert").arg(&a).arg(&out).args(["--jpeg-format", bad]).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "--jpeg-format {bad}");
        assert!(err.contains("--jpeg-format") && err.contains("expected baseline, optimized or progressive"), "--jpeg-format {bad}: {err}");
        assert!(!out.exists(), "--jpeg-format {bad} wrote a file");
    }
}

/// #492: an `--out` folder that is the `--in` folder, however it is spelt, is refused before
/// anything is written, unless `--in-place` asks for it.
#[test]
fn batch_refuses_to_write_over_its_inputs() {
    let d = tmp("batch-in-place");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    write_png(&input.join("a.png"), 8, 4, 3);
    let img = photocraft_codecs::Image::from_u8(8, 4, photocraft_codecs::ChannelLayout::Rgb, vec![90; 96]).unwrap();
    std::fs::write(input.join("a.tif"), photocraft_codecs::encode(&img, photocraft_codecs::Format::Tiff, &Default::default()).unwrap()).unwrap();
    let actions = d.join("actions.json");
    std::fs::write(&actions, r#"[{"command":"image.adjustments.invert"}]"#).unwrap();
    let snapshot = || {
        let mut files: Vec<(PathBuf, Vec<u8>)> =
            std::fs::read_dir(&input).unwrap().map(|e| e.unwrap().path()).map(|p| (p.clone(), std::fs::read(p).unwrap())).collect();
        files.sort();
        files
    };
    let before = snapshot();
    let batch = |out: &Path, extra: &[&str]| {
        bin().current_dir(&d).args(["batch", "--actions"]).arg(&actions).args(["--in", "in", "--out"]).arg(out).args(extra).output().unwrap()
    };
    #[cfg(unix)]
    std::os::unix::fs::symlink(&input, d.join("link")).unwrap();
    let link = cfg!(unix).then(|| PathBuf::from("link"));
    for out in [PathBuf::from("in"), PathBuf::from("./in/"), input.clone(), input.join(".")].iter().chain(&link) {
        for extra in [&[][..], &["--format", "tif"][..]] {
            let o = batch(out, extra);
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(o.status.code(), Some(1), "--out {} {extra:?}: {err}", out.display());
            assert!(err.contains("is the --in folder") && err.contains("--in-place"), "{err}");
            assert_eq!(snapshot(), before, "--out {} {extra:?} changed the originals", out.display());
        }
    }
    // A separate folder runs as before, leaving the originals alone.
    let out = ok(bin().current_dir(&d).args(["batch", "--actions"]).arg(&actions).args(["--in", "in", "--out", "out"])).stdout;
    assert!(out.contains("2 succeeded, 0 failed"), "{out}");
    assert_eq!(snapshot(), before);
    // `--in-place` opts in: each result replaces its original.
    let o = batch(Path::new("in"), &["--in-place"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let after = snapshot();
    assert_eq!(after.len(), 2);
    assert!(after.iter().zip(&before).all(|(a, b)| a.0 == b.0 && a.1 != b.1), "both inputs replaced");
}

/// #7437: `convert` to its own input file, however the output is spelt, is refused before
/// anything is read or written, unless `--in-place` asks for it (as `batch` does, #492).
#[test]
fn convert_refuses_to_write_over_its_input() {
    let d = tmp("convert-in-place");
    write_png(&d.join("in.png"), 8, 4, 5);
    let before = std::fs::read(d.join("in.png")).unwrap();
    let convert = |out: &Path, extra: &[&str]| bin().current_dir(&d).args(["convert", "in.png"]).arg(out).args(extra).output().unwrap();
    let mut outs = vec![PathBuf::from("in.png"), PathBuf::from("./in.png"), d.join("in.png"), d.join(".").join("in.png")];
    std::fs::hard_link(d.join("in.png"), d.join("hard.png")).unwrap();
    outs.push(PathBuf::from("hard.png"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(d.join("in.png"), d.join("soft.png")).unwrap();
        outs.push(PathBuf::from("soft.png"));
    }
    for out in &outs {
        for extra in [&[][..], &["--format", "png"][..], &["--quality", "80"][..]] {
            let o = convert(out, extra);
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(o.status.code(), Some(1), "{} {extra:?}: {err}", out.display());
            assert!(err.contains("is the input file") && err.contains("--in-place"), "{err}");
            assert_eq!(std::fs::read(d.join("in.png")).unwrap(), before, "{} {extra:?} changed the original", out.display());
        }
    }
    // Another file converts as before, leaving the original alone.
    ok(bin().current_dir(&d).args(["convert", "in.png", "out.jpg"]));
    assert!(d.join("out.jpg").is_file());
    assert_eq!(std::fs::read(d.join("in.png")).unwrap(), before);
    // `--in-place` opts in: the result replaces the original, which still opens.
    ok(bin().current_dir(&d).args(["convert", "in.png", "in.png", "--in-place"]));
    let Printed { stdout: out, .. } = ok(bin().current_dir(&d).args(["info", "in.png", "--compact"]));
    let info: Value = serde_json::from_str(&out).unwrap();
    assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(8), Some(4)), "{info}");
}

#[test]
fn commands_listing() {
    let out = ok(bin().arg("commands")).stdout;
    assert!(out.contains("file.new"));
    let out = ok(bin().args(["commands", "--json", "--filter", "layer.new"])).stdout;
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(
        v.as_array()
            .unwrap()
            .iter()
            .all(|c| { c["id"].as_str().unwrap().contains("layer.new") || c["label"].as_str().unwrap().to_lowercase().contains("layer.new") })
    );
}

#[test]
fn in_process_run_matches_binary() {
    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = photocraft_cli::run(["commands", "--filter", "file.new"], &mut out, &mut err);
    assert_eq!(code, 0);
    assert!(String::from_utf8(out).unwrap().contains("file.new"));
}

#[test]
fn mcp_stdio_handshake_and_tool_call() {
    let mut child = bin().arg("mcp").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut send = |v: Value| mcp_send(&mut stdin, v);
    let mut recv = |id: u64| mcp_recv(&mut stdout, id);
    send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}));
    let init = recv(1);
    assert_eq!(init["result"]["serverInfo"]["name"], "photocraft", "{init}");
    send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}));
    let tools = recv(2);
    let names: Vec<&str> = tools["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(names.contains(&"command_run") && names.contains(&"doc_render_preview"), "{names:?}");
    send(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"doc_new","arguments":{"width":8,"height":8}}}));
    let r = recv(3);
    assert_ne!(r["result"]["isError"], true, "{r}");
    send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"doc_render_preview","arguments":{}}}));
    let r = recv(4);
    assert_eq!(r["result"]["content"][0]["type"], "image", "{r}");
    let _ = child.kill();
    let _ = child.wait();
}

/// Send one JSON-RPC line to an `mcp` child.
fn mcp_send(stdin: &mut impl Write, v: Value) {
    writeln!(stdin, "{v}").unwrap();
    stdin.flush().unwrap();
}

/// Read an `mcp` child's lines up to the reply to `id`.
fn mcp_recv(stdout: &mut impl BufRead, id: u64) -> Value {
    loop {
        let mut line = String::new();
        assert!(stdout.read_line(&mut line).unwrap() > 0, "server closed");
        let v: Value = serde_json::from_str(&line).unwrap();
        if v["id"] == id {
            return v;
        }
    }
}

/// `mcp --bridge` takes a port or anything with a `:`; anything else is a usage error. A bare port
/// means 127.0.0.1, and the bridge connects at the first call, so with no app there the call fails
/// naming that address.
#[test]
fn mcp_bridge_takes_a_port_or_an_address() {
    let o = bin().args(["mcp", "--bridge", "nope"]).stdin(Stdio::null()).output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(2), "{err}");
    assert!(err.contains("invalid value 'nope' for '--bridge"), "{err}");
    assert!(err.contains("expected a port, 0-65535 (--bridge also takes 127.0.0.1:<PORT>)"), "{err}");

    // Nothing listens on port 1 in a test run. The token only has to be well-formed.
    let token = "a".repeat(64);
    let mut child = bin()
        .args(["mcp", "--bridge", "1", "--control-token", token.as_str()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    mcp_send(
        &mut stdin,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}),
    );
    mcp_recv(&mut stdout, 1);
    mcp_send(&mut stdin, json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    mcp_send(&mut stdin, json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"session_list","arguments":{}}}));
    let r = mcp_recv(&mut stdout, 2);
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap_or_default().contains("127.0.0.1:1"), "{r}");
    let _ = child.kill();
    let _ = child.wait();
}

/// `--format` mapping two inputs to one name fails the second instead of overwriting the first (#420).
#[test]
fn batch_fails_inputs_that_share_an_output_name() {
    let d = tmp("batch-collide");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    write_png(&input.join("a.png"), 8, 4, 3);
    // Same stem in other formats; `A.tif` differs only in case.
    let img = photocraft_codecs::Image::from_u8(8, 4, photocraft_codecs::ChannelLayout::Rgb, vec![90; 96]).unwrap();
    for (name, format) in [("a.bmp", photocraft_codecs::Format::Bmp), ("A.tif", photocraft_codecs::Format::Tiff)] {
        std::fs::write(input.join(name), photocraft_codecs::encode(&img, format, &Default::default()).unwrap()).unwrap();
    }
    let actions = d.join("actions.json");
    std::fs::write(&actions, r#"[{"command":"image.adjustments.invert"}]"#).unwrap();
    let out_dir = d.join("out");
    let o = bin().args(["batch", "--actions"]).arg(&actions).arg("--in").arg(&input).arg("--out").arg(&out_dir).args(["--format", "png"]).output().unwrap();
    let (out, err) = (String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    assert!(
        !o.status.success(),
        "a lost result must fail the run
{out}
{err}"
    );
    assert!(out.contains("1 succeeded, 2 failed"), "{out}");
    assert_eq!(err.matches("not written").count(), 2, "{err}");
    assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 1);
    // Keeping each file's format, every input gets its own output.
    let out_dir = d.join("same");
    let out = ok(bin().args(["batch", "--actions"]).arg(&actions).arg("--in").arg(&input).arg("--out").arg(&out_dir)).stdout;
    assert!(out.contains("3 succeeded, 0 failed"), "{out}");
}

#[test]
fn droplet_runs_an_action_on_files() {
    let d = tmp("droplet");
    write_png(&d.join("a.png"), 8, 4, 3);
    write_png(&d.join("b.png"), 6, 2, 5);
    let droplet = d.join("rot.pcdroplet");
    std::fs::write(
        &droplet,
        json!({"photocraftDroplet": 1, "name": "rot", "action": {"steps": [["image.imageRotation.90cw", {}]]}, "options": {"format": "png"}}).to_string(),
    )
    .unwrap();
    let out_dir = d.join("out");
    let out = ok(bin().arg("droplet").arg(&droplet).arg(d.join("a.png")).arg(d.join("b.png")).arg("--out").arg(&out_dir)).stdout;
    assert_eq!(out.lines().filter(|l| l.starts_with("ok")).count(), 2, "{out}");
    let img = photocraft_codecs::decode(&std::fs::read(out_dir.join("a.png")).unwrap()).unwrap();
    assert_eq!(img.dimensions(), (4, 8));
    let o = bin().arg("droplet").arg(d.join("a.png")).arg(d.join("b.png")).output().unwrap();
    assert!(!o.status.success(), "not a droplet");
}

#[test]
fn tiff_output_is_flat_unless_tiff_layers_is_given() {
    let d = tmp("tiff-layers");
    let layer_count = |path: &std::path::Path| -> usize {
        let info = ok(bin().arg("info").arg(path)).stdout;
        let v: Value = serde_json::from_str(&info).unwrap();
        v["layers"].as_array().unwrap().len()
    };
    for (name, flag, layers) in [("flat.tif", None, 1), ("layered.tif", Some("--tiff-layers"), 2)] {
        let out = d.join(name);
        let mut cmd = bin();
        cmd.args(["run", "--new", r#"{"width":16,"height":16}"#]).args(["--cmd", "layer.new.layer", "--params", r#"{"name":"Ink"}"#]);
        if let Some(flag) = flag {
            cmd.arg(flag);
        }
        ok(cmd.arg("--out").arg(&out));
        assert_eq!(layer_count(&out), layers, "{name}");
    }
    // convert honours the same flag.
    let src = d.join("layered.tif");
    let flat = d.join("converted.tif");
    ok(bin().arg("convert").arg(&src).arg(&flat));
    assert_eq!(layer_count(&flat), 1);
    let kept = d.join("kept.tif");
    ok(bin().arg("convert").arg(&src).arg(&kept).arg("--tiff-layers"));
    assert_eq!(layer_count(&kept), 2);
    std::fs::remove_dir_all(d).unwrap();
}

/// Issue #1108: a non-Unicode argument (a Latin-1 file name on Linux) used to panic in
/// `std::env::args()` before anything was printed. clap reads the arguments as they are: a path
/// takes one, and text (`--filter`) refuses it with a usage error.
#[cfg(unix)]
#[test]
fn a_non_unicode_path_is_taken_and_non_unicode_text_is_a_usage_error() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let d = tmp("non-unicode");
    let name = OsStr::from_bytes(b"caf\xe9.png");
    // No such file yet: the handler runs and fails on it (exit 1), no usage error.
    for args in [vec![OsStr::new("info"), name], vec![OsStr::new("convert"), name, OsStr::new("out.png")]] {
        let o = bin().current_dir(&d).args(&args).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{args:?}: {err}");
        assert!(!err.contains("panicked"), "{err}");
    }
    // Linux file systems take the name (macOS ones refuse it). `convert` reads it; `info` opens
    // through the automation layer, which still needs UTF-8 paths, and says so (exit 1).
    if cfg!(target_os = "linux") {
        write_png(&d.join(name), 8, 4, 3);
        ok(bin().current_dir(&d).arg("convert").arg(name).arg("out.png"));
        assert!(d.join("out.png").is_file());
        let o = bin().current_dir(&d).arg("info").arg(name).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{err}");
        assert!(err.contains("UTF-8"), "{err}");
    }
    let o = bin().args([OsStr::new("commands"), OsStr::new("--filter"), name]).output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!((o.status.code(), o.stdout.is_empty()), (Some(2), true), "{err}");
    assert!(err.contains("UTF-8") && !err.contains("panicked"), "{err}");
    std::fs::remove_dir_all(d).unwrap();
}

/// `batch --actions set.atn`: a Photoshop Actions file plays like an Actions set, and its steps
/// PhotoCraft can't play are warned about up front.
#[test]
fn batch_plays_a_photoshop_actions_file() {
    use photocraft_engine::actions_cmds::Action;
    let d = tmp("atn");
    let input = d.join("in");
    std::fs::create_dir_all(&input).unwrap();
    write_png(&input.join("a.png"), 8, 4, 3);
    let invert = Action::new("Negative", vec![("image.adjustments.invert".into(), json!({})), ("select.all".into(), json!({}))]);
    let odd = Action::new("Odd", vec![(photocraft_engine::atn_cmds::PHOTOSHOP_STEP.into(), json!({"event": "syntheticFill", "name": "Generative Fill"}))]);
    let (bytes, skipped) = photocraft_engine::atn_cmds::export("Set", &[&invert, &odd], &[], false).unwrap();
    assert!(skipped.is_empty());
    let atn = d.join("Set.atn");
    std::fs::write(&atn, bytes).unwrap();
    let out_dir = d.join("out");
    // Two actions: the set needs --action.
    let o = bin().args(["batch", "--actions"]).arg(&atn).arg("--in").arg(&input).arg("--out").arg(&out_dir).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--action"), "{}", String::from_utf8_lossy(&o.stderr));
    let err = ok(bin().args(["batch", "--actions"]).arg(&atn).args(["--action", "Negative", "--in"]).arg(&input).arg("--out").arg(&out_dir)).stderr;
    assert!(out_dir.join("a.png").exists());
    assert!(!err.contains("warning"), "{err}");
    // The unsupported step is warned about, and fails the file as Photoshop would stop on it.
    let o = bin().args(["batch", "--actions"]).arg(&atn).args(["--action", "Odd", "--in"]).arg(&input).arg("--out").arg(d.join("out2")).output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("Generative Fill"), "{err}");
    assert!(!o.status.success());
}

/// What `photocraft_cli::run` returned and wrote, in-process.
struct Invoke {
    code: i32,
    stdout: String,
    stderr: String,
}

fn invoke(args: &[&str]) -> Invoke {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let code = photocraft_cli::run(args, &mut stdout, &mut stderr);
    Invoke { code, stdout: String::from_utf8(stdout).unwrap(), stderr: String::from_utf8(stderr).unwrap() }
}

const SUBCOMMANDS: [&str; 8] = ["convert", "info", "run", "batch", "droplet", "commands", "mcp", "serve"];

/// Pins top-level dispatch: help and the version go to stdout (exit 0), no arguments is a usage
/// error on stderr. There is no `version` subcommand.
#[test]
fn top_level_help_version_and_no_arguments() {
    let Invoke { code, stdout: out, stderr: err } = invoke(&[]);
    assert_eq!((code, out.is_empty(), err.is_empty()), (2, true, false));
    for help in ["--help", "-h", "help"] {
        let Invoke { code, stdout: out, stderr: err } = invoke(&[help]);
        assert_eq!((code, out.is_empty(), err.is_empty()), (0, false, true), "{help}");
    }
    for version in ["--version", "-V"] {
        let Invoke { code, stdout: out, stderr: err } = invoke(&[version]);
        assert_eq!((code, err.as_str()), (0, ""), "{version}");
        assert!(out.starts_with("photocraft-cli ") && out.ends_with('\n') && out.lines().count() == 1, "{version}: {out}");
    }
    let Invoke { code, stdout: out, stderr: err } = invoke(&["version"]);
    assert_eq!((code, out.is_empty()), (2, true));
    assert!(err.contains("version"), "{err}");
    let Invoke { code, stdout: out, stderr: err } = invoke(&["frobnicate"]);
    assert_eq!((code, out.is_empty()), (2, true));
    assert!(err.contains("frobnicate"), "{err}");
}

/// `<sub> --help` / `-h` prints help on stdout and returns without running the subcommand
/// (`mcp` and `serve` would otherwise block on stdin).
#[test]
fn every_subcommand_help_returns_on_stdout() {
    for sub in SUBCOMMANDS {
        for help in ["--help", "-h"] {
            let Invoke { code, stdout: out, stderr: err } = invoke(&[sub, help]);
            assert_eq!((code, out.is_empty(), err.is_empty()), (0, false, true), "{sub} {help}: {err}");
            // `commands` would also exit 0 with output if it ran; its list names `file.new`, help doesn't.
            assert!(!out.contains("file.new"), "{sub} {help} ran instead of printing help: {out}");
        }
    }
}

/// Every subcommand refuses an unknown flag (bare or `=value`) with exit 2, naming it on stderr.
#[test]
fn every_subcommand_rejects_an_unknown_flag() {
    for sub in SUBCOMMANDS {
        for flag in [&["--fromat"][..], &["--fromat=jpg"][..], &["--fromat", "jpg"][..]] {
            let args: Vec<&str> = [sub].into_iter().chain(flag.iter().copied()).collect();
            let Invoke { code, stdout: out, stderr: err } = invoke(&args);
            assert_eq!((code, out.is_empty()), (2, true), "{args:?}: {err}");
            assert!(err.contains("--fromat"), "{args:?}: {err}");
        }
    }
}

/// `--flag=value`, a bare flag given a value, a value flag without one, repeated flags, and values
/// that start with `-`.
#[test]
fn flag_value_forms_and_repeats() {
    let ids = |out: &str| out.lines().filter_map(|l| l.split_whitespace().next()).map(str::to_owned).collect::<Vec<_>>();
    let Invoke { code, stdout: eq, stderr: err } = invoke(&["commands", "--filter=file.new"]);
    assert_eq!(code, 0, "{err}");
    let spaced = invoke(&["commands", "--filter", "file.new"]).stdout;
    assert_eq!(eq, spaced);
    assert!(ids(&eq).contains(&"file.new".to_string()), "{eq}");
    // A repeated flag is a usage error naming it, more likely a mistake than a choice.
    for repeat in [&["commands", "--filter", "zzz-no-match", "--filter", "file.new"][..], &["commands", "--json", "--json"][..]] {
        let Invoke { code, stdout: out, stderr: err } = invoke(repeat);
        assert_eq!((code, out.is_empty()), (2, true), "{repeat:?}");
        assert!(err.contains(repeat[1]) && err.contains("cannot be used multiple times"), "{repeat:?}: {err}");
    }
    let Invoke { code, stderr: err, .. } = invoke(&["convert", "a.png", "b.jpg", "--quality", "80", "--quality", "60"]);
    assert_eq!(code, 2);
    assert!(err.contains("--quality"), "{err}");
    // A value starting with `-` needs `=`; without it the token is a flag, never the value. A
    // negative number is still a value.
    let Invoke { code, stdout: out, stderr: err } = invoke(&["commands", "--filter=-zzz-no-match"]);
    assert_eq!((code, out.as_str()), (0, ""), "{err}");
    for value in ["-zzz-no-match", "--json"] {
        let Invoke { code, stdout: out, .. } = invoke(&["commands", "--filter", value]);
        assert_eq!((code, out.is_empty()), (2, true), "--filter {value}");
    }
    let Invoke { code, stderr: err, .. } = invoke(&["commands", "--filter", "-1"]);
    assert_eq!(code, 0, "{err}");
    // A bare flag takes no value; a value flag needs one.
    let Invoke { code, stdout: out, stderr: err } = invoke(&["commands", "--json=1"]);
    assert_eq!((code, out.is_empty()), (2, true));
    assert!(err.contains("--json"), "{err}");
    let Invoke { code, stdout: out, stderr: err } = invoke(&["commands", "--filter"]);
    assert_eq!((code, out.is_empty()), (2, true));
    assert!(err.contains("--filter"), "{err}");
}

/// Each `--params` applies to the `--cmd` before it; a `--cmd` without one gets `{}`; a
/// `--params` ahead of every `--cmd` fails naming `--params`.
#[test]
fn run_pairs_params_with_the_preceding_cmd() {
    let Invoke { code, stdout: out, stderr: err } = invoke(&[
        "run",
        "--new",
        r#"{"width":8,"height":8}"#,
        "--cmd",
        "layer.new.layer",
        "--params",
        r#"{"name":"First"}"#,
        "--cmd",
        "layer.new.layer",
        "--cmd",
        "layer.new.layer",
        "--params",
        r#"{"name":"Third"}"#,
        "--cmd",
        "document.inspect",
    ]);
    assert_eq!(code, 0, "{err}");
    let rows: Vec<Value> = out.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(rows.len(), 4, "{out}");
    let names: Vec<&str> = rows[3]["result"]["layers"].as_array().unwrap().iter().filter_map(|l| l["name"].as_str()).collect();
    let count = |n: &str| names.iter().filter(|x| **x == n).count();
    assert_eq!((count("First"), count("Third")), (1, 1), "each --params applies to its own --cmd only: {names:?}");
    // Three new layers plus the background; the middle one (no --params) has a default name.
    assert_eq!(names.len(), 4, "{names:?}");
    // Layers list top first: Third, the unnamed middle one, First, Background. Pairing --params
    // with --cmd by position (params[i] -> cmd[i]) would name the middle one Third instead.
    assert_eq!((names[0], names[2]), ("Third", "First"), "{names:?}");
    let Invoke { code, stderr: err, .. } = invoke(&["run", "--new", "{}", "--params", "{}", "--cmd", "layer.new.layer"]);
    assert_ne!(code, 0);
    assert!(err.contains("--params"), "{err}");
}

/// #7789: `convert` opens a developed raw at Raw Defaults › Resolution (default 300 ppi), as
/// `run`, `info` and the app do; it wrote 72.
#[test]
fn convert_opens_a_raw_at_the_raw_defaults_resolution() {
    use photocraft_raw::testgen::{DngSpec, mosaic, scene};
    let d = tmp("convert-raw");
    let (w, h) = (24, 16);
    let (raw, psd) = (d.join("IMG_0001.dng"), d.join("IMG_0001.psd"));
    std::fs::write(&raw, DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build()).unwrap();
    ok(bin().args(["convert"]).arg(&raw).arg(&psd));
    let doc = photocraft_io::import("IMG_0001.psd", &std::fs::read(&psd).unwrap()).unwrap().document;
    assert_eq!(doc.resolution_dpi, 300.0);
    std::fs::remove_dir_all(d).unwrap();
}

/// `--out=-image.png` writes `-image.png`; `--out -image.png` is a usage error and writes nothing.
#[test]
fn a_value_starting_with_a_dash_is_written_with_equals() {
    let d = tmp("dash-value");
    let o = bin().current_dir(&d).args(["run", "--new", r#"{"width":4,"height":4}"#, "--out", "-image.png"]).output().unwrap();
    assert_eq!(o.status.code(), Some(2), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
    ok(bin().current_dir(&d).args(["run", "--new", r#"{"width":4,"height":4}"#, "--out=-image.png"]));
    assert!(d.join("-image.png").is_file());
    std::fs::remove_dir_all(d).unwrap();
}

/// `run` takes exactly one of `<FILE>` or `--new`, and `mcp`/`serve` one of `--control-token` or
/// `--control-token-file`: anything else is a usage error naming the flags, before any work.
#[test]
fn either_or_flags_are_usage_errors() {
    let Invoke { code, stderr: err, .. } = invoke(&["run", "a.png", "--new", "{}", "--cmd", "layer.new.layer"]);
    assert_eq!(code, 2);
    assert!(err.contains("--new") && err.contains("cannot be used with"), "{err}");
    let Invoke { code, stderr: err, .. } = invoke(&["run", "--cmd", "layer.new.layer"]);
    assert_eq!(code, 2);
    assert!(err.contains("--new"), "{err}");
    let Invoke { code, stderr: err, .. } = invoke(&["run", "--new", "{}", "--cmd", "layer.new.layer", "--params", "{"]);
    assert_eq!(code, 2);
    assert!(err.contains("--params"), "{err}");
    let token = "a".repeat(64);
    for mode in [["mcp", "--bridge", "1"], ["serve", "--port", "0"]] {
        let o = bin().args(mode).args(["--control-token", token.as_str(), "--control-token-file", "token"]).stdin(Stdio::null()).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "{mode:?}: {err}");
        assert!(err.contains("--control-token-file") && err.contains("cannot be used with"), "{mode:?}: {err}");
    }
    for port in ["70000", "x"] {
        let o = bin().args(["serve", "--port", port]).stdin(Stdio::null()).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "--port {port}: {err}");
        assert!(err.contains("--port"), "--port {port}: {err}");
    }
}

/// How a run of the binary ended.
struct Exited {
    code: Option<i32>,
    stderr: String,
}

/// The binary run with `args` and one variable set, stdin at its end.
fn with_var(args: &[&str], var: &str, value: &str) -> Exited {
    let o = bin().args(args).env(var, value).stdin(Stdio::null()).output().unwrap();
    Exited { code: o.status.code(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
}

/// A port nothing listens on, for a `serve` to take.
fn free_port() -> u16 {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port()
}

/// `serve` on TCP never exits: read its stderr until it says it is serving (or its stderr closes),
/// then kill it.
fn serve_tcp_stderr(cmd: &mut Command) -> String {
    use std::io::Read as _;
    let mut child = cmd.stdin(Stdio::null()).stderr(Stdio::piped()).spawn().unwrap();
    // Read on a thread, so a `serve` that stays silent with stderr open fails the test after the
    // deadline instead of hanging it.
    let mut stderr = child.stderr.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stderr.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut err = String::new();
    while !err.contains("photocraft-cli serving on") {
        match rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            Ok(chunk) => err.push_str(&String::from_utf8_lossy(&chunk)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    err
}

/// #2491: `PHOTOCRAFT_CONTROL_PORT` turns `serve` from stdio into a TCP listener and `mcp` from a
/// headless engine into a bridge, and both say so on stderr. A flag wins, an empty value counts as
/// unset, and a bad value is a usage error naming the variable.
#[test]
fn control_port_env_switches_modes_with_a_notice_and_flags_win() {
    let port = free_port();
    let err = serve_tcp_stderr(bin().arg("serve").env("PHOTOCRAFT_CONTROL_PORT", port.to_string()));
    assert!(err.contains(&format!("PHOTOCRAFT_CONTROL_PORT={port} is set; serving on TCP instead of stdio")), "{err}");
    assert!(err.contains("unset it to serve stdio") && err.contains("photocraft-cli serving on"), "{err}");

    // No token is given, so the bridge fails after the notice.
    let err = with_var(&["mcp"], "PHOTOCRAFT_CONTROL_PORT", &port.to_string()).stderr;
    assert!(err.contains(&format!("PHOTOCRAFT_CONTROL_PORT is set; bridging to the app on 127.0.0.1:{port}")), "{err}");
    assert!(err.contains("unset it to run headless"), "{err}");

    // The flag wins, so the variable's bad value is never parsed.
    let err = serve_tcp_stderr(bin().args(["serve", "--port"]).arg(port.to_string()).env("PHOTOCRAFT_CONTROL_PORT", "not-a-port"));
    assert!(err.contains("photocraft-cli serving on") && !err.contains("PHOTOCRAFT_CONTROL_PORT"), "{err}");

    // Empty or blank counts as unset: `serve` reads stdio (at its end, so it exits at once) and
    // `mcp` stays headless.
    // A root on the command line is the headless engine's: the variable can't switch to a bridge
    // that would drop it.
    for root in ["--automation-read-root", "--automation-write-root"] {
        let Exited { code, stderr: err } = with_var(&["mcp", root, "."], "PHOTOCRAFT_CONTROL_PORT", &port.to_string());
        assert_eq!(code, Some(2), "{root}: {err}");
        assert!(err.contains(root) && err.contains("cannot be used with 'PHOTOCRAFT_CONTROL_PORT'") && !err.contains("bridging"), "{root}: {err}");
    }

    for empty in ["", "  "] {
        let Exited { code, stderr: err } = with_var(&["serve"], "PHOTOCRAFT_CONTROL_PORT", empty);
        assert!(code == Some(0) && !err.contains("PHOTOCRAFT_CONTROL_PORT"), "{empty:?}: {err}");
        let err = with_var(&["mcp"], "PHOTOCRAFT_CONTROL_PORT", empty).stderr;
        assert!(!err.contains("bridging"), "{empty:?}: {err}");
    }

    for (sub, bad) in [
        ("serve", "not-a-port"),
        ("serve", "70000"),
        ("mcp", "nope"),
        ("mcp", "70000"),
        // `--bridge` takes host:port, the variable only a port.
        ("mcp", "127.0.0.1:7878"),
        ("mcp", ":7878"),
        ("mcp", "bad:port"),
    ] {
        let Exited { code, stderr: err } = with_var(&[sub], "PHOTOCRAFT_CONTROL_PORT", bad);
        assert_eq!(code, Some(2), "{sub} {bad}: {err}");
        assert!(err.contains(&format!("invalid value '{bad}' for 'PHOTOCRAFT_CONTROL_PORT'")), "{sub} {bad}: {err}");
    }
}

/// `--control-token` and `--control-token-file` fall back to `PHOTOCRAFT_CONTROL_TOKEN` and
/// `PHOTOCRAFT_CONTROL_TOKEN_FILE`. One from the environment and the other as a flag is the
/// either/or usage error, naming the variable, when a token is used: a stale exported pair doesn't
/// stop headless `mcp` or stdio `serve`. Help names the token's variable, never its value.
#[test]
fn control_token_env_vars_are_the_flags_fallbacks() {
    let token = "a".repeat(64);
    for mode in [["mcp", "--bridge", "1"], ["serve", "--port", "0"]] {
        let Exited { code, stderr: err } = with_var(&[&mode[..], &["--control-token", token.as_str()][..]].concat(), "PHOTOCRAFT_CONTROL_TOKEN_FILE", "token");
        assert_eq!(code, Some(2), "{mode:?}: {err}");
        assert!(err.contains("PHOTOCRAFT_CONTROL_TOKEN_FILE") && err.contains("cannot be used with"), "{mode:?}: {err}");
        let Exited { code, stderr: err } = with_var(&[&mode[..], &["--control-token-file", "token"][..]].concat(), "PHOTOCRAFT_CONTROL_TOKEN", &token);
        assert_eq!(code, Some(2), "{mode:?}: {err}");
        assert!(err.contains("PHOTOCRAFT_CONTROL_TOKEN") && err.contains("cannot be used with") && !err.contains(&token), "{mode:?}: {err}");
    }
    for sub in ["mcp", "serve"] {
        let o = bin().arg(sub).env("PHOTOCRAFT_CONTROL_TOKEN", &token).env("PHOTOCRAFT_CONTROL_TOKEN_FILE", "token").stdin(Stdio::null()).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(!err.contains("cannot be used with"), "{sub}: {err}");
        // stdio `serve` exits 0 at the end of its input; how headless `mcp` ends there isn't
        // pinned, so it only must not be the usage error.
        if sub == "serve" {
            assert_eq!(o.status.code(), Some(0), "{sub}: {err}");
        } else {
            assert_ne!(o.status.code(), Some(2), "{sub}: {err}");
        }
        let o = bin().args([sub, "--help"]).env("PHOTOCRAFT_CONTROL_TOKEN", &token).output().unwrap();
        let out = String::from_utf8_lossy(&o.stdout);
        assert!(out.contains("[env: PHOTOCRAFT_CONTROL_TOKEN]") && !out.contains(&token), "{sub}: {out}");
    }
    // The server takes the variable's token, so it neither generates nor prints one; an empty
    // variable counts as unset.
    let err = serve_tcp_stderr(bin().args(["serve", "--port"]).arg(free_port().to_string()).env("PHOTOCRAFT_CONTROL_TOKEN", &token));
    assert!(err.contains("photocraft-cli serving on") && !err.contains("control token"), "{err}");
    let err = serve_tcp_stderr(bin().args(["serve", "--port"]).arg(free_port().to_string()).env("PHOTOCRAFT_CONTROL_TOKEN", ""));
    assert!(err.contains("photocraft-cli serving on") && err.contains("photocraft-cli control token: "), "{err}");
}

/// #8802: Deselect with nothing selected is skipped in a `run`, as action playback does (#6744),
/// so a script that deselects after Reveal Selection still finishes and saves.
#[test]
fn run_skips_deselect_with_nothing_selected() {
    let folder = tmp("deselect-run");
    let native = folder.join("out.pcraft").to_string_lossy().to_string();
    let Invoke { code, stdout, stderr } = invoke(&[
        "run",
        "--new",
        r#"{"width":16,"height":16}"#,
        "--cmd",
        "select.rect",
        "--params",
        r#"{"x":0,"y":0,"width":8,"height":16}"#,
        "--cmd",
        "layer.new.layer",
        "--cmd",
        "layer.layerMask.revealSelection",
        "--cmd",
        "select.deselect",
        "--cmd",
        "layer.new.layer",
        "--params",
        r#"{"name":"after"}"#,
        "--out",
        &native,
    ]);
    assert_eq!(code, 0, "{stderr}");
    let results: Vec<Value> = stdout.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(results.len(), 5, "every command reports a result: {stdout}");
    assert_eq!(results[3], json!({"command": "select.deselect", "result": null}));
    let Invoke { code, stdout, stderr } = invoke(&["info", &native, "--compact"]);
    assert_eq!(code, 0, "{stderr}");
    let info: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(info["layers"][0]["name"], "after", "the steps after Deselect ran: {info}");

    // Deselect with a selection still deselects: Inverse afterwards finds nothing to invert.
    let Invoke { code, stderr, .. } = invoke(&[
        "run",
        "--new",
        r#"{"width":16,"height":16}"#,
        "--cmd",
        "select.all",
        "--cmd",
        "select.deselect",
        "--cmd",
        "select.inverse",
        "--out",
        &native,
    ]);
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("select.inverse") && stderr.contains("no selection"), "{stderr}");
    let _ = std::fs::remove_dir_all(&folder);
}

/// #8802: a batched action that starts with Deselect plays on files with nothing selected, as the
/// same action does in the Actions panel (#6744).
#[test]
fn batch_skips_deselect_with_nothing_selected() {
    let dir = tmp("deselect-batch");
    let path = |name: &str| dir.join(name).to_string_lossy().to_string();
    std::fs::create_dir_all(dir.join("in")).unwrap();
    let Invoke { code, stderr, .. } = invoke(&["run", "--new", r#"{"width":8,"height":6}"#, "--out", &path("in/a.png")]);
    assert_eq!(code, 0, "{stderr}");
    std::fs::write(dir.join("steps.json"), json!([["select.deselect", {}], ["image.imageRotation.90cw", {}]]).to_string()).unwrap();
    let Invoke { code, stdout, stderr } = invoke(&["batch", "--actions", &path("steps.json"), "--in", &path("in"), "--out", &path("out")]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("1 succeeded, 0 failed"), "{stdout}");
    let Invoke { code, stdout, stderr } = invoke(&["info", &path("out/a.png"), "--compact"]);
    assert_eq!(code, 0, "{stderr}");
    let info: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(6), Some(8)), "the step after Deselect ran: {info}");
    let _ = std::fs::remove_dir_all(&dir);
}
