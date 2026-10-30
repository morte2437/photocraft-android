use super::*;
use crate::cmd::smart_cmds::source::{flattened_source, source_surface_from_buffer};

/// A source document of `mode`/`depth` with a textured, partly transparent block on a layer.
fn source(mode: ColorMode, depth: SampleType) -> Document {
    let mut doc = Document::new("source", Size::new(W as u32, H as u32), mode, depth);
    doc.layers = vec![Layer::new("block", LayerContent::Raster(block(&doc.pixel_format(), Rect::new(4, 3, 50, 40))))];
    doc
}

fn block(fmt: &PixelFormat, r: Rect) -> Surface {
    let mut s = Surface::new(*fmt);
    let mut data = Vec::new();
    for y in r.y0..r.y1 {
        for x in r.x0..r.x1 {
            let v = ((x * 7 + y * 13) % 23) as f32 / 22.0;
            let a = if (x + y) % 5 == 0 { 0.5 } else { 1.0 };
            data.extend(photocraft_raster::from_rgba(fmt, [v, 1.0 - v, ((x * y) % 7) as f32 / 6.0, a]));
        }
    }
    s.write_region(r, &data);
    s
}

/// Everything the smart object cache would hold, bit for bit.
fn pixels(s: &Surface, bounds: Rect) -> Vec<u32> {
    s.read_region(bounds).iter().map(|v| v.to_bits()).collect()
}

/// The composite through the full-size RGBA buffer, the way sources were rendered before.
fn through_buffer(doc: &Document, fmt: PixelFormat) -> Surface {
    source_surface_from_buffer(doc, &photocraft_compose::flatten(doc), fmt).unwrap()
}

fn same(label: &str, doc: &Document, fmt: PixelFormat) {
    let (old, new) = (through_buffer(doc, fmt), flattened_source(doc, fmt).unwrap());
    assert_eq!(new.format(), old.format(), "{label}: format");
    assert!(pixels(&old, doc.bounds()) == pixels(&new, doc.bounds()), "{label}: pixels differ");
}

#[test]
fn rgb_sources_match_at_every_depth() {
    for depth in SampleType::ALL {
        let doc = source(ColorMode::Rgb, depth);
        same(&format!("rgb {depth:?}"), &doc, doc.pixel_format());
    }
}

#[test]
fn a_gray_source_in_an_rgb_document_and_back_matches() {
    let gray = source(ColorMode::Grayscale, SampleType::U8);
    same("gray into rgb", &gray, PixelFormat { mode: ColorMode::Rgb, sample: SampleType::U8, alpha: true });
    let rgb = source(ColorMode::Rgb, SampleType::U8);
    same("rgb into gray", &rgb, PixelFormat { mode: ColorMode::Grayscale, sample: SampleType::U8, alpha: true });
}

#[test]
fn an_8_bit_source_in_a_32_bit_document_matches() {
    let doc = source(ColorMode::Rgb, SampleType::U8);
    same("8 into 32", &doc, PixelFormat { mode: ColorMode::Rgb, sample: SampleType::F32, alpha: true });
}

/// The banded compositor composites CMYK in inks and Lab in L, a and b, and lands up to 16 and 1
/// steps (8-bit) off the RGBA buffer on visible pixels, so these sources keep the buffer path.
#[test]
fn a_cmyk_source_matches() {
    let doc = source(ColorMode::Cmyk, SampleType::U8);
    same("cmyk", &doc, doc.pixel_format());
}

#[test]
fn a_lab_source_matches() {
    let doc = source(ColorMode::Lab, SampleType::U8);
    same("lab", &doc, doc.pixel_format());
}

#[test]
fn a_layered_source_with_a_mask_blend_mode_and_effect_matches() {
    for depth in [SampleType::U8, SampleType::F32] {
        let mut doc = source(ColorMode::Rgb, depth);
        let fmt = doc.pixel_format();
        let mut top = Layer::new("top", LayerContent::Raster(block(&fmt, Rect::new(20, 10, 60, 46))));
        top.blend = BlendMode::Multiply;
        top.opacity = 0.7;
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(30, 0, 64, 48), &[0.25]);
        top.mask = Some(mask);
        doc.layers.push(top);
        let mut shadowed = Layer::new("shadowed", LayerContent::Raster(block(&fmt, Rect::new(8, 20, 24, 36))));
        shadowed.effects.items.push(photocraft_doc::Effect::default_drop_shadow());
        shadowed.effects.enabled = true;
        doc.layers.push(shadowed);
        same(&format!("layered {depth:?}"), &doc, fmt);
    }
}

#[test]
fn a_source_past_the_render_limit_is_refused_not_emptied() {
    let doc = Document::new("huge", Size::new(60_000, 60_000), ColorMode::Rgb, SampleType::U8);
    assert!(flattened_source(&doc, doc.pixel_format()).is_err());
}

#[test]
fn an_rgb_source_in_a_cmyk_or_lab_document_matches() {
    let doc = source(ColorMode::Rgb, SampleType::U8);
    for mode in [ColorMode::Cmyk, ColorMode::Lab] {
        same(&format!("rgb into {mode:?}"), &doc, PixelFormat { mode, sample: SampleType::U8, alpha: true });
    }
}

/// Blocks are 256 rows of the full width at 4096 px, and split at x = 4096 at 5000 px: the shadow
/// and mask cross the block edges both ways.
#[test]
fn effects_across_block_edges_match() {
    for (width, depth) in [(4096, SampleType::U8), (4096, SampleType::F32), (5000, SampleType::U8)] {
        let mut doc = Document::new("wide", Size::new(width, 700), ColorMode::Rgb, depth);
        let fmt = doc.pixel_format();
        let x0 = if width > 4096 { 3800 } else { 100 };
        let mut base = Layer::new("base", LayerContent::Raster(block(&fmt, Rect::new(x0, 200, x0 + 800, 560))));
        let mut glow = photocraft_doc::Effect::default_drop_shadow();
        if let photocraft_doc::Effect::DropShadow(d) = &mut glow {
            d.size = 40.0;
            d.distance = 30.0;
        }
        base.effects.items.push(glow);
        base.effects.enabled = true;
        let mut mask = LayerMask::reveal_all();
        mask.surface.fill_rect(Rect::new(0, 240, width as i32, 270), &[0.3]);
        base.mask = Some(mask);
        doc.layers = vec![base];
        same(&format!("blocks {width} {depth:?}"), &doc, fmt);
    }
}
