//! A Photomerge result is named "Untitled_Panorama-N", N being the first number no open
//! document uses, so "Untitled_Panorama-1" when no panorama is open (#14093).

#![cfg(not(target_arch = "wasm32"))]

use super::*;

/// An RGB document `name` of `w`x`h` cut at `x0` from a `sw`-wide noise image.
fn tile(name: &str, sc: &[[f32; 4]], sw: usize, x0: usize, w: usize, h: usize) -> Document {
    let mut d = Document::new(name, Size::new(w as u32, h as u32), ColorMode::Rgb, SampleType::U8);
    let fmt = d.pixel_format();
    let n = fmt.channels();
    let mut data = vec![0.0f32; w * h * n];
    for (i, o) in data.chunks_exact_mut(n).enumerate() {
        photocraft_raster::from_rgba_into(&fmt, sc[(i / w) * sw + x0 + i % w], o);
    }
    let mut surf = Surface::new(fmt);
    surf.write_region(Rect::new(0, 0, w as i32, h as i32), &data);
    d.layers.push(Layer::new("Background", LayerContent::Raster(surf)));
    d
}

/// A deterministic textured RGB image: a gradient with scattered squares.
fn scene(w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut z = 7u64;
    let mut rnd = || {
        z = z.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (z >> 33) as f64 / (1u64 << 31) as f64
    };
    let mut px: Vec<[f32; 4]> = (0..w * h).map(|i| [0.3 + 0.2 * (i % w) as f32 / w as f32, 0.35, 0.4, 1.0]).collect();
    for _ in 0..(w * h / 600) {
        let (cx, cy, r) = ((rnd() * w as f64) as usize, (rnd() * h as f64) as usize, 2 + (rnd() * 8.0) as usize);
        let col = [rnd() as f32, rnd() as f32, rnd() as f32, 1.0];
        for y in cy.saturating_sub(r)..(cy + r).min(h) {
            for x in cx.saturating_sub(r)..(cx + r).min(w) {
                px[y * w + x] = col;
            }
        }
    }
    px
}

/// Two 300x300 tiles cut at x 0 and 200 from one 500x300 image, written as PNG files; returns
/// their paths and the directory guard that removes them when dropped.
fn source_files() -> (tempfile::TempDir, Vec<String>) {
    let (sw, h) = (500usize, 300usize);
    let sc = scene(sw, h);
    let dir = tempfile::tempdir().unwrap();
    let paths = [("a.png", 0usize), ("b.png", 200)]
        .into_iter()
        .map(|(file, x0)| {
            let png = photocraft_io::export(&tile(file, &sc, sw, x0, 300, h), file, &Default::default()).unwrap().bytes;
            let path = dir.path().join(file);
            std::fs::write(&path, &png).unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect();
    (dir, paths)
}

/// Runs Photomerge on `paths` and returns the result's name.
fn merge(s: &mut Session, paths: &[String]) -> String {
    let r = s.execute("file.automate.photomerge", json!({"paths": paths, "layout": "reposition"})).unwrap();
    assert_eq!(r["placed"].as_array().unwrap().len(), 2, "{r}");
    s.active().unwrap().doc.name.clone()
}

fn blank(name: &str) -> Document {
    Document::new(name, Size::new(10, 10), ColorMode::Rgb, SampleType::U8)
}

#[test]
fn panorama_is_untitled_panorama_1_whatever_else_is_open() {
    let (_dir, paths) = source_files();
    for open in [0, 1, 3] {
        let mut s = Session::new();
        for k in 0..open {
            s.add_document(blank(&format!("doc{k}")), None);
        }
        assert_eq!(merge(&mut s, &paths), "Untitled_Panorama-1", "{open} other documents open");
    }
}

#[test]
fn a_second_panorama_gets_another_number() {
    let (_dir, paths) = source_files();
    let mut s = Session::new();
    assert_eq!(merge(&mut s, &paths), "Untitled_Panorama-1");
    // The first panorama is still open, so the next one takes the next free number.
    assert_eq!(merge(&mut s, &paths), "Untitled_Panorama-2");
    // A free number below the taken ones is reused.
    let mut s = Session::new();
    s.add_document(blank("Untitled_Panorama-2"), None);
    assert_eq!(merge(&mut s, &paths), "Untitled_Panorama-1");
}
