//! The command's mask read, algorithm input/output and tiled replacement share this window.

use photocraft_doc::Document;
use photocraft_geom::{Rect, TILE_SIZE};

pub(super) fn area(doc: &Document, radius: f32) -> Rect {
    let canvas = doc.bounds();
    let Some(selection) = doc.selection.as_ref() else { return Rect::EMPTY };
    // content_bounds describes differences from the default. A nonzero default means
    // implicit selected pixels throughout the canvas, including with no allocated tiles.
    if selection.default_pixel().first().is_some_and(|v| *v != 0.0) {
        return canvas;
    }
    // Raw tile coordinates can come from document data. content_bounds sorts by i32
    // coordinate differences, which can overflow outside the pixel-addressable range.
    // The prior canvas reader only queries valid coordinates and safely ignores these.
    let valid = (i32::MIN / TILE_SIZE)..=(i32::MAX / TILE_SIZE);
    if selection.tiles().any(|(tile, _)| !valid.contains(&tile.tx) || !valid.contains(&tile.ty)) {
        return canvas;
    }
    let bounds = selection.content_bounds().intersect(&canvas);
    if bounds.is_empty() {
        return Rect::EMPTY;
    }
    let halo = (radius.ceil() as i32).saturating_add(1);
    bounds.inflate(halo).intersect(&canvas)
}

#[cfg(test)]
mod tests;
