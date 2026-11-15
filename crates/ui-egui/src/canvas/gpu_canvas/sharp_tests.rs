//! #11122: the sharp filter (crisp texels with a one-device-pixel edge) depends only on where and
//! how large the stored texels are drawn, not on how much a viewport page was reduced. Drawn
//! through the production callback and shader on a real device, with synthetic pixels.

use super::*;
use photocraft_compose::Buffer;
use photocraft_doc::SampleType;
use photocraft_geom::Rect;

/// Texels per side of the test texture.
const TEXELS: usize = 4;
/// Side of each drawn rect, in device pixels; its centre is the view centre.
const CELL: usize = 80;

/// A 4 × 4 texture, black before texel `edge` and white from it, across columns (`vertical`) or
/// rows.
fn edge_texture(edge: usize, vertical: bool) -> Buffer {
    let mut buf = Buffer::filled(Rect::from_xywh(0, 0, TEXELS as u32, TEXELS as u32), [0.0, 0.0, 0.0, 1.0]);
    for (i, px) in buf.px.iter_mut().enumerate() {
        let at = if vertical { i % TEXELS } else { i / TEXELS };
        if at >= edge {
            *px = [1.0, 1.0, 1.0, 1.0];
        }
    }
    buf
}

fn view(doc: u64, doc_size: [u32; 2], zoom: f32) -> ViewParams {
    let center = [doc_size[0] as f32 / 2.0, doc_size[1] as f32 / 2.0];
    ViewParams {
        doc,
        doc_size,
        zoom,
        center,
        rotation: 0.0,
        shadow: false,
        pixel_grid: false,
        view_key: doc,
        display: 0,
        output: 0,
        hdr: None,
        offset: [0.0, 0.0],
    }
}

/// One comparison: the same texels as a page reduced by `factor` and at full resolution, both
/// drawn at `per_texel` device pixels per stored texel.
struct Case {
    factor: u32,
    vertical: bool,
    per_texel: f32,
    edge: usize,
}

impl Case {
    /// Where the black-to-white edge falls along the scanned axis, in cell pixels.
    fn edge_at(&self) -> f32 {
        CELL as f32 / 2.0 - (TEXELS as f32 / 2.0 - self.edge as f32) * self.per_texel
    }
}

#[test]
fn sharp_edges_match_at_every_page_reduction() {
    let _gpu = photocraft_testkit::gpu_lock();
    let rs = std::panic::catch_unwind(|| egui_kittest::wgpu::create_render_state(wgpu_setup(), Default::default()));
    let Ok(rs) = rs else {
        eprintln!("skipping sharp filter readback: no GPU adapter");
        return;
    };
    let gpu = GpuCanvas::new(&rs);
    // Every reduction a viewport page uses up to 8, for an edge across columns and across rows.
    // The document scale is 2 (aligned) or 2 + 0.5 / factor (fractional), so both draws take the
    // sharp path (`filter_mode`). Aligned: the edge at texel 2 lands on the cell centre, a pixel
    // boundary. Fractional: 2 × factor + 0.5 px per texel puts the edge at texel 1 inside a pixel,
    // where exactly one pixel shows the ramp (nearest sampling would show none).
    let mut cases = Vec::new();
    for factor in [2u32, 4, 8] {
        for vertical in [true, false] {
            cases.push(Case { factor, vertical, per_texel: 2.0 * factor as f32, edge: 2 });
            cases.push(Case { factor, vertical, per_texel: 2.0 * factor as f32 + 0.5, edge: 1 });
        }
    }
    let side = TEXELS as u32;
    let mut draws = Vec::new();
    for (i, c) in cases.iter().enumerate() {
        let tex = edge_texture(c.edge, c.vertical);
        let (reduced, full) = (2 * i as u64 + 1, 2 * i as u64 + 2);
        let reduced_size = [side * c.factor, side * c.factor];
        for (key, size, factor) in [(reduced, reduced_size, c.factor), (full, [side, side], 1)] {
            let core = Rect::new(0, 0, size[0] as i32, size[1] as i32);
            assert!(gpu.upload_page(PageUpload { key, page: 1, size, depth: SampleType::U8, factor, core, buf: &tex }), "page {key}");
        }
        draws.push((view(reduced, reduced_size, c.per_texel / c.factor as f32), view(full, [side, side], c.per_texel)));
    }
    let renderer = egui_kittest::wgpu::WgpuTestRenderer::from_render_state(rs);
    // One row per case: the reduced page in the left cell, the full one in the right.
    let cell = CELL as f32;
    let mut h =
        egui_kittest::Harness::builder().with_size(egui::vec2(2.0 * cell, cell * cases.len() as f32)).with_pixels_per_point(1.0).renderer(renderer).build_ui(
            move |ui| {
                let o = ui.ctx().content_rect().min;
                for (i, (reduced, full)) in draws.iter().enumerate() {
                    let at = |col: f32| egui::Rect::from_min_size(o + egui::vec2(col * cell, i as f32 * cell), egui::vec2(cell, cell));
                    GpuCanvas::paint(ui.painter(), at(0.0), *reduced);
                    GpuCanvas::paint(ui.painter(), at(1.0), *full);
                }
            },
        );
    h.run_steps(2);
    let img = h.render().expect("render");
    assert!(gpu.health.is_ok(), "{:?}", gpu.fault());
    for (i, c) in cases.iter().enumerate() {
        let what = format!("factor {}, {} edge, {} px per texel", c.factor, if c.vertical { "vertical" } else { "horizontal" }, c.per_texel);
        // The image spans the cell centre ± 2 texels; scan across the edge inside it, on lines
        // through and either side of the centre.
        let half = (TEXELS as f32 / 2.0 * c.per_texel) as usize;
        let (lo, hi) = (CELL / 2 - half + 1, CELL / 2 + half - 1);
        let edge = c.edge_at();
        for line in [CELL / 2 - 6, CELL / 2, CELL / 2 + 5] {
            let px = |col: usize, along: usize| {
                let (x, y) = if c.vertical { (along, line) } else { (line, along) };
                img.get_pixel((col * CELL + x) as u32, (i * CELL + y) as u32).0
            };
            for along in lo..hi {
                let (reduced, full) = (px(0, along), px(1, along));
                for ch in 0..3 {
                    // Identical texels at an identical place and size: equal up to rounding.
                    assert!(reduced[ch].abs_diff(full[ch]) <= 1, "{what}, line {line}, at {along}: reduced {reduced:?}, full {full:?}");
                }
                // Exactly one device pixel of ramp, where the edge falls: none when it lands on
                // a pixel boundary, the pixel it crosses when it lands inside one.
                let centre = along as f32 + 0.5;
                let v = full[0];
                if centre < edge {
                    assert!(v < 16, "{what}, line {line}: dark before the edge at {edge}, {along} is {v}");
                } else if centre > edge {
                    assert!(v > 239, "{what}, line {line}: light after the edge at {edge}, {along} is {v}");
                } else {
                    assert!((16..240).contains(&v), "{what}, line {line}: the pixel the edge crosses at {edge} shows the ramp, {v}");
                }
            }
        }
    }
}
