//! Feathered marquee / lasso on a 6000×4000 (24 MP) canvas: the pre-windowing reference
//! (shape → dense full-canvas mask → `sel::feather` → `sel::combine(None, …, Replace)`)
//! against the current `select.rect` / `select.lasso` commands, whose dense feather mask
//! stays a (50 + 2·(⌈3·4⌉+1))² = 76² = 5776-pixel window around the 50×50 shape.
//! `cargo run --release -p photocraft-engine --example bench_tool_feather [reps]`
use std::time::{Duration, Instant};

use photocraft_algo::selection as sel;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, Size};
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const W: i32 = 6000;
const H: i32 = 4000;
/// A 50×50 shape centred on the canvas, feathered by 4 px.
const X: i32 = W / 2 - 25;
const Y: i32 = H / 2 - 25;
const SHAPE: i32 = 50;
const FEATHER: f32 = 4.0;
const DEFAULT_REPS: usize = 7;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Rect,
    Ellipse,
    Lasso,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Rect => "rect",
            Kind::Ellipse => "ellipse",
            Kind::Lasso => "lasso",
        }
    }

    fn command(self) -> &'static str {
        match self {
            Kind::Lasso => "select.lasso",
            Kind::Rect | Kind::Ellipse => "select.rect",
        }
    }

    fn params(self) -> Value {
        let r = json!({"x": X, "y": Y, "width": SHAPE, "height": SHAPE, "feather": FEATHER});
        match self {
            Kind::Rect => r,
            Kind::Ellipse => json!({"x": X, "y": Y, "width": SHAPE, "height": SHAPE, "ellipse": true, "feather": FEATHER}),
            Kind::Lasso => json!({"points": [[X, Y], [X + SHAPE, Y], [X + SHAPE, Y + SHAPE], [X, Y + SHAPE]], "feather": FEATHER}),
        }
    }

    /// The path the windowed commands replaced: shape construction, a dense full-canvas f32
    /// mask, the full-canvas blur, and a full-canvas `Replace` combine against no old selection.
    fn reference(self, area: Rect) -> Option<Surface> {
        let mask = match self {
            Kind::Rect | Kind::Ellipse => {
                let r = Rect::from_xywh(X, Y, SHAPE as u32, SHAPE as u32);
                let shape = if self == Kind::Ellipse {
                    sel::ellipse_surface(r, area, true)
                } else {
                    let mut s = Surface::new(PixelFormat::GRAY8);
                    s.fill_rect(r.intersect(&area), &[1.0]);
                    s
                };
                sel::mask_from_surface(Some(&shape), area)
            }
            // Raw floating-point coverage: never quantise before the blur.
            Kind::Lasso => {
                let e = (X + SHAPE) as f32;
                let (x, y, ey) = (X as f32, Y as f32, (Y + SHAPE) as f32);
                sel::polygon(&[(x, y), (e, y), (e, ey), (x, ey)], area, true)
            }
        };
        let (w, h) = (area.width() as usize, area.height() as usize);
        let mask = sel::feather(&mask, w, h, FEATHER);
        sel::combine(None, &mask, area, sel::SelectionMode::Replace)
    }
}

fn median(times: &[Duration]) -> Duration {
    let mut sorted = times.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let Some(mid) = sorted.get(n / 2).copied() else { return Duration::ZERO };
    if n % 2 == 1 { mid } else { sorted.get(n / 2 - 1).copied().map_or(mid, |prev| (prev + mid) / 2) }
}

fn main() -> Res<()> {
    let reps = match std::env::args().nth(1) {
        None => DEFAULT_REPS,
        Some(v) => {
            let n: usize = v.parse().map_err(|e| format!("reps `{v}` must be a whole number ({e})"))?;
            if !(1..=1000).contains(&n) {
                return Err("reps must be in 1..=1000".into());
            }
            n
        }
    };
    // Sparse 24 MP document: `Document::new` plus an empty raster layer allocates no pixels.
    let mut doc = Document::new("bench-24mp", Size::new(W as u32, H as u32), ColorMode::Grayscale, SampleType::U8);
    let format = doc.pixel_format();
    doc.layers.push(Layer::raster("Layer 1", format));
    let mut s = Session::new();
    s.add_document(doc, None);
    let area = s.active().map(|st| st.doc.bounds()).ok_or("no document")?;

    let full_px = W as u64 * H as u64;
    let side = SHAPE as u64 + 2 * ((FEATHER * 3.0).ceil() as u64 + 1);
    println!("tool feather, 6000×4000 = {full_px} px canvas, {SHAPE}×{SHAPE} shape, feather {FEATHER} px, median of {reps}");
    println!(
        "dense f32 mask: full canvas {full_px} px / {} B  vs  windowed {side}×{side} = {} px / {} B ({}× fewer)\n",
        full_px * 4,
        side * side,
        side * side * 4,
        full_px / (side * side),
    );
    for kind in [Kind::Rect, Kind::Ellipse, Kind::Lasso] {
        // One untimed pass each: allocator and first-call setup stay out of the medians.
        drop(std::hint::black_box(kind.reference(area)));
        let _ = s.execute("select.deselect", json!({}));
        s.execute(kind.command(), kind.params())?;
        let mut reference = Vec::with_capacity(reps);
        for _ in 0..reps {
            let t = Instant::now();
            let out = kind.reference(area);
            reference.push(t.elapsed());
            std::hint::black_box(&out);
            drop(out);
        }
        let mut current = Vec::with_capacity(reps);
        for _ in 0..reps {
            let _ = s.execute("select.deselect", json!({}));
            let t = Instant::now();
            s.execute(kind.command(), kind.params())?;
            current.push(t.elapsed());
        }
        let (r, c) = (median(&reference), median(&current));
        let (ms, cm) = (r.as_secs_f64() * 1000.0, c.as_secs_f64() * 1000.0);
        let speedup = if cm > 0.0 { ms / cm } else { f64::INFINITY };
        println!("{:<6} reference {ms:>9.2} ms   current {cm:>9.2} ms   speedup {speedup:>7.1}x", kind.label());
    }
    Ok(())
}
