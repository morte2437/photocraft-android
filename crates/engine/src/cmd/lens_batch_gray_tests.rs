//! File › Automate › Lens Correction hands back a Grayscale 8-bit source as RGB 8-bit, as
//! Photoshop does; other modes and depths keep today's behaviour (#13990).
#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use photocraft_color::{ColorMode, SampleType};
use serde_json::json;

use super::LENS_BATCH;
use crate::Session;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pc-lens-gray-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("in")).unwrap();
    dir
}

/// Writes a 40 x 30 source in `mode` and `depth` to `<dir>/in/<file>`.
fn source(dir: &Path, file: &str, mode: &str, depth: u32) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 40, "height": 30, "mode": mode, "depth": depth})).unwrap();
    let path = dir.join("in").join(file);
    crate::file_cmds::save_doc(&s.active().unwrap().doc, path.to_str().unwrap(), None).unwrap();
}

/// Runs the batch on `<dir>/in` with `format` and returns the mode and depth of `<dir>/out/<file>`.
fn run(dir: &Path, format: &str, file: &str) -> (ColorMode, SampleType) {
    let mut s = Session::new();
    let r = s.execute(LENS_BATCH, json!({"input": dir.join("in").to_str().unwrap(), "output": dir.join("out").to_str().unwrap(), "format": format})).unwrap();
    assert!(r["errors"].as_array().unwrap().is_empty(), "{r}");
    let path = dir.join("out").join(file);
    let doc = photocraft_io::import(file, &std::fs::read(&path).unwrap()).unwrap().document;
    (doc.mode, doc.depth)
}

#[test]
fn a_grayscale_8_bit_source_comes_out_rgb_8_bit_in_every_file_type() {
    for format in ["psd", "tif", "png", "jpg"] {
        let dir = temp_dir(format);
        source(&dir, "gray.psd", "grayscale", 8);
        assert_eq!(run(&dir, format, &format!("gray.{format}")), (ColorMode::Rgb, SampleType::U8), "{format}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn rgb_sources_keep_their_depth() {
    for (format, depth, sample) in [("psd", 8, SampleType::U8), ("psd", 16, SampleType::U16), ("tif", 8, SampleType::U8), ("tif", 16, SampleType::U16)] {
        let dir = temp_dir(&format!("rgb{depth}-{format}"));
        source(&dir, "rgb.psd", "rgb", depth);
        assert_eq!(run(&dir, format, &format!("rgb.{format}")), (ColorMode::Rgb, sample), "{format} {depth}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn other_modes_and_depths_keep_their_mode() {
    for (mode, depth, expected) in [("grayscale", 16, (ColorMode::Grayscale, SampleType::U16)), ("cmyk", 8, (ColorMode::Cmyk, SampleType::U8))] {
        let dir = temp_dir(&format!("{mode}{depth}"));
        source(&dir, "src.psd", mode, depth);
        assert_eq!(run(&dir, "psd", "src.psd"), expected, "{mode} {depth}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
