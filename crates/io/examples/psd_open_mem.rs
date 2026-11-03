//! Opens a file the way the app's background open does and reports the time. Run it under
//! `/usr/bin/time -l` for the peak memory. `--read` reads the whole file first and imports the
//! bytes, as opens did before `import_file` (to compare).
//!
//! `cargo run --release -p photocraft-io --example psd_open_mem -- <file.psd> [--read]`

fn main() {
    let Some(path) = std::env::args().nth(1).filter(|a| a != "--read") else {
        eprintln!("usage: psd_open_mem <file.psd>");
        std::process::exit(2);
    };
    if std::env::args().any(|a| a == "--read") {
        read_then_import(&path);
        return;
    }
    let t = std::time::Instant::now();
    let res =
        match photocraft_io::import_file(&path, std::path::Path::new(&path), &photocraft_io::ImportOptions::default(), &photocraft_raster::Interrupt::NONE) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{path}: open failed: {e}");
                std::process::exit(1);
            }
        };
    report(&path, &format!("open {:.2}s", t.elapsed().as_secs_f64()), &res);
}

fn read_then_import(path: &str) {
    let t = std::time::Instant::now();
    let bytes = match photocraft_format::read_file(std::path::Path::new(path)) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("{path}: read failed: {e}");
            std::process::exit(1);
        }
    };
    let read_s = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let res = match photocraft_io::import_with(path, &bytes, &photocraft_raster::Interrupt::NONE) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{path}: open failed: {e}");
            std::process::exit(1);
        }
    };
    drop(bytes);
    report(path, &format!("read {read_s:.2}s, import {:.2}s", t.elapsed().as_secs_f64()), &res);
}

fn report(path: &str, times: &str, res: &photocraft_io::ImportResult) {
    let doc = &res.document;
    println!("{path}: {times}, {} layers ({} top level)", doc.walk().len(), doc.layers.len());
    for w in &res.warnings {
        println!("  warning: {w}");
    }
}
