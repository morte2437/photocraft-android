//! Large-file fixtures (#216, #5107), generated at test time by `photocraft_psd::fixtures`: a 20 MP
//! layered PSD, a 150-layer layout, a PSD just under Photoshop's 2 GB cap and a 4.4 GB PSB. Each
//! is opened through `photocraft_io`, checked (size, layers, masks, effects, merged pixels), saved
//! again and re-opened.
//!
//! The two layered documents run in memory; the streamed files are written to
//! `$PHOTOCRAFT_FIXTURE_DIR` (default: the system temp dir), removed afterwards, and are opt-in:
//! `cargo test --release -p photocraft-io --test it -- large_files:: --ignored`.

use std::path::PathBuf;

use photocraft_doc::{Document, LayerContent};
use photocraft_geom::Rect;
use photocraft_psd::Version;
use photocraft_psd::fixtures::{Grid, LAYERED_20MP, LAYOUT_150, PSB_4GB, PSD_2GB, Streamed};

/// Raster layers, groups, masked layers, layers with effects and hidden layers in `doc`.
fn census(doc: &Document) -> [usize; 5] {
    let mut n = [0; 5];
    for (_, _, l) in doc.walk() {
        match l.content {
            LayerContent::Raster(_) => n[0] += 1,
            LayerContent::Group(_) => n[1] += 1,
            _ => {}
        }
        n[2] += usize::from(l.mask.is_some());
        n[3] += usize::from(!l.effects.items.is_empty());
        n[4] += usize::from(!l.visible);
    }
    n
}

fn check_grid(g: &Grid, doc: &Document, what: &str) {
    assert_eq!((doc.size.width, doc.size.height), (g.width, g.height), "{what}");
    let layers: Vec<_> = (0..g.layers).filter_map(|i| g.layer(i)).collect();
    let want = [
        g.layers as usize + 1,
        g.rows() as usize,
        layers.iter().filter(|l| l.masked).count(),
        layers.iter().filter(|l| l.shadow).count(),
        layers.iter().filter(|l| l.hidden).count(),
    ];
    assert_eq!(census(doc), want, "{what}: [raster, groups, masked, with effects, hidden]");
    for (x, y) in g.sample_points() {
        let px = photocraft_compose::render(doc, Rect::from_xywh(x as i32, y as i32, 1, 1)).px;
        let got = px.first().copied().unwrap_or_default();
        let want = g.expected(x, y);
        for c in 0..3 {
            let v = (got[c] * 255.0).round() as i32;
            assert!((v - i32::from(want[c])).abs() <= 1, "{what}: ({x}, {y}) channel {c}: got {v}, want {}", want[c]);
        }
        assert!(got[3] > 0.999, "{what}: ({x}, {y}) is not opaque");
    }
}

/// Builds `g`, opens it, checks it, saves it as PSD and checks the re-opened file.
fn grid_roundtrip(g: Grid, what: &str) {
    let t = std::time::Instant::now();
    let bytes = g.build(Version::Psd).and_then(|f| f.to_bytes()).unwrap();
    let gen_s = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    let res = photocraft_io::import("fixture.psd", &bytes).unwrap();
    let open_s = t.elapsed().as_secs_f64();
    check_grid(&g, &res.document, what);
    let out = photocraft_io::export(&res.document, "fixture.psd", &Default::default()).unwrap();
    let again = photocraft_io::import("fixture.psd", &out.bytes).unwrap();
    check_grid(&g, &again.document, &format!("{what}, saved and re-opened"));
    eprintln!(
        "{what}: {:.1} MB, generated in {gen_s:.2} s, opened in {open_s:.2} s, saved as {:.1} MB",
        bytes.len() as f64 / 1e6,
        out.bytes.len() as f64 / 1e6
    );
}

/// 150 layers in 15 groups on A4 at 300 ppi, with masks, hidden layers, closed groups and drop
/// shadows.
#[test]
fn layout_with_150_layers_opens_and_saves() {
    grid_roundtrip(LAYOUT_150, "150-layer layout");
}

/// A 20 MP layered photo: background, five layers in two groups, a mask and a drop shadow.
#[test]
fn layered_20_mp_psd_opens_and_saves() {
    grid_roundtrip(LAYERED_20MP, "20 MP layered");
}

fn fixture_dir() -> PathBuf {
    let dir = std::env::var_os("PHOTOCRAFT_FIXTURE_DIR").map_or_else(std::env::temp_dir, PathBuf::from).join("photocraft-large-files");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Removes the file when the test ends, passed or not.
struct Remove(PathBuf);

impl Drop for Remove {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn check_streamed(case: &Streamed, doc: &Document, what: &str) {
    assert_eq!((doc.size.width, doc.size.height), (case.width, case.height), "{what}");
    assert_eq!(doc.layers.len(), 1, "{what}");
    let s = doc.layers[0].surface().unwrap();
    let (w, h) = (case.width, case.height);
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1), (w / 2, h / 2), (w / 3, h * 2 / 3), (12_345 % w, h - 7)] {
        let got = s.pixel(x as i32, y as i32);
        assert_eq!(got.len(), 4, "{what}");
        for (c, v) in got.iter().enumerate() {
            let want = f32::from(case.sample(x, y, c as u32)) / 255.0;
            assert!((v - want).abs() <= 1.0 / 510.0, "{what}: ({x}, {y}) channel {c}: got {v}, want {want}");
        }
    }
}

/// Writes `case`, opens it, checks it, saves it again (the writer picks PSD or PSB) and checks the
/// re-opened file.
fn streamed_roundtrip(case: &Streamed) {
    let path = fixture_dir().join(case.file_name());
    let _cleanup = Remove(path.clone());
    let t = std::time::Instant::now();
    let len = case.write_file(&path).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), len);
    eprintln!("{}: {:.2} GB written in {:.1} s", case.name, len as f64 / 1e9, t.elapsed().as_secs_f64());

    let t = std::time::Instant::now();
    let bytes = std::fs::read(&path).unwrap();
    let res = photocraft_io::import(&case.file_name(), &bytes).unwrap();
    drop(bytes);
    eprintln!("{}: read and opened in {:.1} s, warnings {:?}", case.name, t.elapsed().as_secs_f64(), res.warnings);
    check_streamed(case, &res.document, case.name);

    let t = std::time::Instant::now();
    let out = photocraft_io::export(&res.document, &case.file_name(), &Default::default()).unwrap();
    drop(res);
    // Version 2 = PSB: a re-save past 2 GB must switch to PSB; a smaller one stays PSD.
    let psb = out.bytes.get(4..6) == Some(&[0, 2]);
    assert!(psb || (case.version == Version::Psd && (out.bytes.len() as u64) < photocraft_psd::fixtures::PSD_MAX_FILE_BYTES), "{}", case.name);
    eprintln!(
        "{}: saved {:.2} GB ({}) in {:.1} s, warnings {:?}",
        case.name,
        out.bytes.len() as f64 / 1e9,
        if psb { "PSB" } else { "PSD" },
        t.elapsed().as_secs_f64(),
        out.warnings
    );
    let again = photocraft_io::import(if psb { "again.psb" } else { "again.psd" }, &out.bytes).unwrap();
    drop(out);
    check_streamed(case, &again.document, &format!("{}, saved and re-opened", case.name));
}

/// A 17,000² layered PSD of 2.02 GB, just under Photoshop's 2 GB PSD cap.
#[test]
#[ignore = "writes a 2 GB file and needs ~12 GB of memory; run in release"]
fn psd_just_under_2_gb_opens_and_saves() {
    streamed_roundtrip(&PSD_2GB);
}

/// A 25,000² layered PSB of 4.38 GB: layer channels of 625 MB, data past 4 GiB (#375).
#[test]
#[ignore = "writes a 4.4 GB file and needs ~25 GB of memory; run in release"]
fn psb_of_4_gb_opens_and_saves() {
    streamed_roundtrip(&PSB_4GB);
}
