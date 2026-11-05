//! A custom Shape Blur kernel is admitted against the working-memory budget before its coverage
//! buffer is allocated, also when a smart filter re-renders it. Its own binary: the budget is
//! process-wide.

use photocraft_geom::Rect;
use photocraft_raster::memory::{self, Policy};
use serde_json::json;

#[test]
fn custom_shape_kernel_is_admitted_before_allocation() {
    let knots = json!([[0, 0], [10, 0], [10, 10], [0, 10]]);
    let params = json!({"radius":1000,"shapePath":{"subpaths":[{"closed":true,"knots":knots}]}});
    let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    surface.write_region(Rect::new(0, 0, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
    let canvas = Rect::new(0, 0, 1, 1);
    let render = || photocraft_engine::filters::apply_filter_to_surface("filter.blur.shapeBlur", &params, &surface, canvas, None, canvas);
    // The default budget renders it.
    assert!(render().unwrap().is_some());
    // A 1 MiB budget cannot hold the 2001² coverage floats (16 MB).
    memory::configure(Policy { percent: None, absolute_bytes: 1 << 20 });
    let refused = render();
    memory::configure(Policy::default());
    let err = refused.expect_err("the kernel must be refused before allocation");
    assert!(err.to_string().contains("working memory"), "{err}");
}
