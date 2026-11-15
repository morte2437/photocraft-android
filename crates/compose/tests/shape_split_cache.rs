use std::sync::Arc;

use photocraft_color::{Color, SampleType};
use photocraft_compose::shape_split;
use photocraft_doc::Fill;
use photocraft_doc::vector::{Path, ShapeLayer, ShapeStroke, Subpath};
use photocraft_geom::Rect;

/// A small working set beyond 32 shapes should reuse its tiles at every sample depth.
#[test]
fn small_shape_working_sets_keep_their_cached_tiles() {
    let canvas = Rect::new(0, 0, 400, 240);
    let shapes: Vec<_> = (0..40)
        .map(|i| {
            let (x, y) = (12.0 + f64::from(i % 8) * 40.0, 12.0 + f64::from(i / 8) * 40.0);
            ShapeLayer {
                path: Path::new(vec![Subpath::polygon(&[(x, y), (x + 20.0, y), (x + 20.0, y + 20.0), (x, y + 20.0)])]),
                fill: Some(Fill::Solid(Color::rgb(0.2, 0.4, 0.6))),
                stroke: Some(ShapeStroke { width: 2.0, ..ShapeStroke::default() }),
                ..ShapeLayer::default()
            }
        })
        .collect();
    for depth in SampleType::ALL {
        shape_split::purge_cache();
        let first: Vec<_> = shapes.iter().map(|shape| shape_split::split(shape, canvas, depth).expect("stroked shape")).collect();
        for (i, (shape, before)) in shapes.iter().zip(&first).enumerate() {
            let after = shape_split::split(shape, canvas, depth).expect("cached shape");
            for (before, after) in [(&before.0, &after.0), (&before.1, &after.1)] {
                assert!(before.tile_count() > 0);
                assert_eq!(before.tile_count(), after.tile_count());
                for (coord, tile) in before.tiles() {
                    assert!(Arc::ptr_eq(tile, after.tile(*coord).expect("cached tile")), "shape {i}, {depth:?}: rerasterized cached pixels");
                }
            }
        }
    }
    shape_split::purge_cache();
}
