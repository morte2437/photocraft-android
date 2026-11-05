//! A rough upper bound of the memory an export needs, so a caller can refuse a save that cannot
//! fit before starting it (#10024): on the web a failed allocation aborts the module and the
//! document is lost.

use std::collections::HashSet;

use photocraft_doc::{Document, LayerContent, SmartSource};
use photocraft_geom::TILE_SIZE;

/// Bytes per pixel of the compositor's working buffer (`RGBA`, `f32`), filled when a flat format
/// is written.
const COMPOSITE_BYTES_PER_PX: u64 = 16;

/// Formats that keep the layers: their model is built from the layer pixels, not a composite.
fn keeps_layers(ext: &str) -> bool {
    matches!(ext, "psd" | "psb" | "pcraft" | "ora" | "kra" | "xcf")
}

/// Formats whose writer fills a composite of the canvas: every flat format, and OpenRaster for
/// its `mergedimage.png`.
fn fills_composite(ext: &str) -> bool {
    !keeps_layers(ext) || ext == "ora"
}

/// The extension of `name_or_ext` (a file name, path or bare extension), lowercase.
fn extension(name_or_ext: &str) -> String {
    let file = name_or_ext.rsplit(['/', '\\']).next().unwrap_or(name_or_ext);
    file.rsplit_once('.').map_or(file, |(_, ext)| ext).to_ascii_lowercase()
}

/// The bytes exporting `doc` to `name_or_ext` may need before anything it allocates can fail
/// cleanly: the output model, about one copy of every layer's and mask's pixel tiles plus the
/// embedded smart-object files (each distinct one counted once); a flat format and OpenRaster
/// also fill a composite of the canvas, and PSD/PSB their merged image ([`merged_image_bytes`]).
/// The encoded file is not counted: its buffer is reserved fallibly (`PsdFile::to_bytes`), so if
/// it doesn't fit the save fails with an error rather than trapping, and counting it would refuse
/// saves that fit. Saturates; never panics.
pub fn estimate_export_bytes(doc: &Document, name_or_ext: &str) -> u64 {
    let tile_bytes = (TILE_SIZE as u64) * (TILE_SIZE as u64) * doc.pixel_format().bytes_per_pixel().max(1) as u64;
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for (_, _, layer) in doc.walk() {
        let tiles = layer.surface().map_or(0, |s| s.tile_count()) + layer.mask.as_ref().map_or(0, |m| m.surface.tile_count());
        total = total.saturating_add((tiles as u64).saturating_mul(tile_bytes));
        if let LayerContent::Smart(so) = &layer.content
            && let SmartSource::Embedded { bytes, .. } = &so.source
            && seen.insert(so.contents_id)
        {
            total = total.saturating_add(bytes.len() as u64);
        }
    }
    let mut need = total;
    let ext = extension(name_or_ext);
    if fills_composite(&ext) {
        need = need.saturating_add(canvas_px(doc).saturating_mul(COMPOSITE_BYTES_PER_PX));
    }
    if matches!(ext.as_str(), "psd" | "psb") {
        need = need.saturating_add(merged_image_bytes(doc));
    }
    need
}

fn canvas_px(doc: &Document) -> u64 {
    u64::from(doc.size.width).saturating_mul(u64::from(doc.size.height))
}

/// PSD and PSB always write a canvas-sized merged image: the real composite with maximum
/// compatibility, a white placeholder without it. Its planes (colour channels plus alpha, in the
/// document's sample size) and their encoded copy are held together while the file is built.
fn merged_image_bytes(doc: &Document) -> u64 {
    let fmt = doc.pixel_format();
    let per_px = (fmt.mode.color_channels() as u64 + 1).saturating_mul(fmt.sample.bytes() as u64);
    canvas_px(doc).saturating_mul(per_px).saturating_mul(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::{ColorMode, SampleType};
    use photocraft_doc::{Layer, SmartObject};
    use photocraft_geom::Affine;
    use std::sync::Arc;

    fn doc_with_smart_object(len: usize) -> Document {
        let mut doc = Document::new("t", photocraft_geom::Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        let source = SmartSource::Embedded { file_name: "a.png".into(), bytes: Arc::new(vec![0; len]) };
        doc.layers.push(Layer::new("so", LayerContent::Smart(SmartObject::new(source, Affine::IDENTITY, None))));
        doc
    }

    #[test]
    fn counts_embedded_files_once_and_adds_a_composite_for_flat_formats() {
        let doc = doc_with_smart_object(10 << 20);
        // RGB 8-bit: three colour planes and alpha, held raw and encoded.
        let merged = 64 * 64 * 4 * 2;
        assert_eq!(estimate_export_bytes(&doc, "x.psd"), (10 << 20) + merged);
        assert_eq!(estimate_export_bytes(&doc, "/a/b/x.PSD"), (10 << 20) + merged);
        assert_eq!(estimate_export_bytes(&doc, "x.pcraft"), 10 << 20);
        assert_eq!(estimate_export_bytes(&doc, "png"), (10 << 20) + 64 * 64 * 16);
        // OpenRaster keeps the layers but also writes a merged image, flattened first.
        assert_eq!(estimate_export_bytes(&doc, "x.ora"), (10 << 20) + 64 * 64 * 16);
    }

    #[test]
    fn a_shared_source_is_counted_once() {
        let mut doc = doc_with_smart_object(1 << 20);
        let dup = doc.layers[0].clone();
        doc.layers.push(dup);
        assert_eq!(estimate_export_bytes(&doc, "psd"), (1 << 20) + 64 * 64 * 4 * 2);
    }

    #[test]
    fn an_empty_document_needs_next_to_nothing() {
        let doc = Document::new("t", photocraft_geom::Size::new(10, 10), ColorMode::Rgb, SampleType::U8);
        assert_eq!(estimate_export_bytes(&doc, "psd"), 10 * 10 * 4 * 2, "only the merged image");
        assert_eq!(estimate_export_bytes(&doc, "pcraft"), 0);
    }
}
