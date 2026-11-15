//! Exercise the recipe runner with the real CLI and a synthetic editable PSD.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

#[test]
fn edited_psd_roundtrip_is_checked_and_bad_expectations_fail() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let base = root.join("base.psd");
    let cli = crate::binary();
    let made = Command::new(&cli)
        .args(["run", "--new", r#"{"width":64,"height":64}"#, "--cmd", "type.create", "--params", r#"{"text":"Oracle","x":4,"y":30,"name":"Oracle"}"#, "--out"])
        .arg(&base)
        .output()
        .unwrap();
    assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
    let hash = xtask::sha256::hex(&fs::read(&base).unwrap());
    let mut recipe = json!({"version":1,"corpus_revision":"synthetic","cases":[{
        "id":"style", "base":"base.psd", "base_sha256":hash, "target":{"kind":"Type","name":"Oracle"},
        "commands":[{"command":"type.antiAlias.crisp","params":{}}],
        "checks":[{"op":"changed_to","path":"/text/antialias","value":"Crisp"},{"op":"unchanged","path":"/text/text"}]
    }]});
    let recipe_path = root.join("recipes.json");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    fs::write(&recipe_path, serde_json::to_vec(&recipe).unwrap()).unwrap();
    let cli_path = Path::new(&cli).to_str().unwrap();
    let corpus = root.to_str().unwrap();
    let recipe_name = recipe_path.to_str().unwrap();
    let output = root.join("success");
    let args = [recipe_name, corpus, cli_path, output.to_str().unwrap()];
    xtask::psd_write_oracle::run(&src, &args).unwrap();
    let report: Value = serde_json::from_slice(&fs::read(output.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["cases"][0]["status"], "pass");
    assert_eq!(report["cases"][0]["photoshop"]["opens"], "not_run");
    recipe["cases"][0]["checks"][0]["value"] = json!("Sharp");
    fs::write(&recipe_path, serde_json::to_vec(&recipe).unwrap()).unwrap();
    let bad_output = root.join("failure");
    assert!(xtask::psd_write_oracle::run(&src, &[recipe_name, corpus, cli_path, bad_output.to_str().unwrap()]).is_err());
    let bad: Value = serde_json::from_slice(&fs::read(bad_output.join("report.json")).unwrap()).unwrap();
    assert_eq!(bad["cases"][0]["status"], "fail");
}
