//! Colour glyphs (COLR v0 and v1 with CPAL) draw in the font's colours (#7711). The fonts are
//! Inter with a COLR and a CPAL table spliced in, so no colour font has to be bundled.

use std::sync::Arc;

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::TextLayer;
use photocraft_doc::text::{AntiAlias, CharStyle, TextRun};
use photocraft_geom::Affine;
use skrifa::{FontRef, MetadataProvider};

use crate::render::{ink_rect, rasterize};
use crate::{TextEngine, TextLayout, fonts};

/// `font` with extra tables, the table directory rebuilt in tag order.
fn with_tables(font: &[u8], extra: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let be16 = |o: usize| u16::from_be_bytes([font[o], font[o + 1]]) as usize;
    let be32 = |o: usize| u32::from_be_bytes([font[o], font[o + 1], font[o + 2], font[o + 3]]) as usize;
    let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..be16(4))
        .map(|i| {
            let r = 12 + 16 * i;
            let (off, len) = (be32(r + 8), be32(r + 12));
            ([font[r], font[r + 1], font[r + 2], font[r + 3]], font[off..off + len].to_vec())
        })
        .collect();
    tables.extend(extra.iter().cloned());
    tables.sort_by_key(|t| t.0);
    let n = tables.len();
    let mut out = font[..4].to_vec();
    let pow = 1usize << (usize::BITS - 1 - n.leading_zeros());
    for v in [n, pow * 16, pow.trailing_zeros() as usize, n * 16 - pow * 16] {
        out.extend_from_slice(&(v as u16).to_be_bytes());
    }
    let offset = 12 + 16 * n;
    let mut data = Vec::new();
    for (tag, body) in &tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&((offset + data.len()) as u32).to_be_bytes());
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        data.extend_from_slice(body);
        data.resize(data.len().next_multiple_of(4), 0);
    }
    out.extend_from_slice(&data);
    out
}

/// A CPAL table with one palette of straight RGBA bytes.
fn cpal(colors: &[[u8; 4]]) -> Vec<u8> {
    let mut t = Vec::new();
    for v in [0u16, colors.len() as u16, 1, colors.len() as u16] {
        t.extend_from_slice(&v.to_be_bytes());
    }
    t.extend_from_slice(&14u32.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());
    for [r, g, b, a] in colors {
        t.extend_from_slice(&[*b, *g, *r, *a]);
    }
    t
}

/// A COLR v0 table: `base` drawn as `layers` (glyph, palette entry), bottom first.
fn colr_v0(base: u16, layers: &[(u16, u16)]) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&1u16.to_be_bytes());
    t.extend_from_slice(&14u32.to_be_bytes());
    t.extend_from_slice(&20u32.to_be_bytes());
    t.extend_from_slice(&(layers.len() as u16).to_be_bytes());
    for v in [base, 0, layers.len() as u16] {
        t.extend_from_slice(&v.to_be_bytes());
    }
    for (g, p) in layers {
        t.extend_from_slice(&g.to_be_bytes());
        t.extend_from_slice(&p.to_be_bytes());
    }
    t
}

/// A COLR v1 table: `base` is its own outline filled with a linear gradient from palette entry 0
/// at x = `x0` to entry 1 at x = `x1` (font units).
fn colr_v1_gradient(base: u16, x0: i16, x1: i16) -> Vec<u8> {
    let mut t = Vec::new();
    // Header: version 1, no v0 records, then the BaseGlyphList right after the 34-byte header.
    t.extend_from_slice(&1u16.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&34u32.to_be_bytes());
    for _ in 0..4 {
        t.extend_from_slice(&0u32.to_be_bytes());
    }
    // BaseGlyphList: one record, its paint 10 bytes in.
    t.extend_from_slice(&1u32.to_be_bytes());
    t.extend_from_slice(&base.to_be_bytes());
    t.extend_from_slice(&10u32.to_be_bytes());
    // PaintGlyph (format 10): the child paint 6 bytes in, clipped to `base`.
    t.extend_from_slice(&[10, 0, 0, 6]);
    t.extend_from_slice(&base.to_be_bytes());
    // PaintLinearGradient (format 4): colour line 16 bytes in; p0, p1, p2 (p2 above p0).
    t.extend_from_slice(&[4, 0, 0, 16]);
    for v in [x0, 0, x1, 0, x0, 1000] {
        t.extend_from_slice(&v.to_be_bytes());
    }
    // ColorLine: pad, two stops (offset, palette entry, alpha), F2Dot14.
    t.push(0);
    t.extend_from_slice(&2u16.to_be_bytes());
    for (offset, entry) in [(0i16, 0u16), (1 << 14, 1)] {
        t.extend_from_slice(&offset.to_be_bytes());
        t.extend_from_slice(&entry.to_be_bytes());
        t.extend_from_slice(&(1i16 << 14).to_be_bytes());
    }
    t
}

fn gid(font: &[u8], c: char) -> u16 {
    let f = FontRef::new(font).expect("Inter parses");
    f.charmap().map(c).map_or(0, |g| g.to_u32() as u16)
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// `text` laid out in Inter at 72 dpi, then drawn with `font` (Inter plus colour tables).
fn layout_with(text: &str, font: Vec<u8>, color: Color) -> TextLayout {
    let mut e = TextEngine::new();
    let style = CharStyle { size_pt: 64.0, color, ..Default::default() };
    let layer = TextLayer { text: text.into(), font_family: "Inter".into(), runs: vec![TextRun { len: text.len(), style }], ..Default::default() };
    let mut l = e.layout(&layer, 72.0);
    for f in &mut l.faces {
        f.font = parley::FontData::new(parley::fontique::Blob::new(Arc::new(font.clone())), 0);
    }
    l
}

/// Straight RGBA of every covered pixel, as RGB whatever the format.
fn pixels(l: &TextLayout, format: PixelFormat) -> Vec<[f32; 4]> {
    let r = rasterize(l, &Affine::IDENTITY, format, AntiAlias::Sharp);
    let data = r.surface.read_region(r.rect);
    let n = format.channels();
    data.chunks_exact(n)
        .map(|p| {
            let a = p[n - 1];
            let rgb = match format.mode {
                ColorMode::Grayscale => [p[0]; 3],
                _ => [p[0], p[1], p[2]],
            };
            [rgb[0], rgb[1], rgb[2], a]
        })
        .collect()
}

fn depths() -> [PixelFormat; 3] {
    [PixelFormat::RGBA8, PixelFormat::RGBA16, PixelFormat::RGBA32F]
}

#[test]
fn colr_v0_layers_take_their_palette_colours_not_the_text_colour() {
    let inter = fonts::INTER_REGULAR.as_slice();
    let (h, i) = (gid(inter, 'H'), gid(inter, 'I'));
    // H is red; an I on top takes the text colour (palette entry 0xFFFF).
    let font = with_tables(inter, &[(*b"COLR", colr_v0(h, &[(h, 0), (i, 0xFFFF)])), (*b"CPAL", cpal(&[RED]))]);
    let green = Color::rgb(0.0, 1.0, 0.0);
    for format in depths() {
        let px = pixels(&layout_with("H", font.clone(), green), format);
        let solid: Vec<_> = px.iter().filter(|p| p[3] > 0.99).collect();
        assert!(!solid.is_empty(), "{format:?}: nothing drawn");
        let red = solid.iter().filter(|p| p[0] > 0.98 && p[1] < 0.02 && p[2] < 0.02).count();
        let fg = solid.iter().filter(|p| p[1] > 0.98 && p[0] < 0.02 && p[2] < 0.02).count();
        assert!(red > 50, "{format:?}: {red} red pixels");
        assert!(fg > 20, "{format:?}: {fg} pixels in the text colour");
        // The rest are the I's anti-aliased edge over the red: mixes of the two, nothing else.
        assert!(solid.len() - red - fg < 20, "{format:?}: {} mixed pixels", solid.len() - red - fg);
        assert!(solid.iter().all(|p| p[2] < 0.02 && (p[0] + p[1] - 1.0).abs() < 0.03), "{format:?}: a colour that is neither");
    }
    // Without the colour tables the same glyph is all text colour.
    let px = pixels(&layout_with("H", inter.to_vec(), green), PixelFormat::RGBA8);
    assert!(px.iter().filter(|p| p[3] > 0.99).all(|p| p[1] > 0.98 && p[0] < 0.02));
}

#[test]
fn colour_glyph_takes_the_text_colours_opacity_and_any_colour_mode() {
    let inter = fonts::INTER_REGULAR.as_slice();
    let h = gid(inter, 'H');
    let font = with_tables(inter, &[(*b"COLR", colr_v0(h, &[(h, 0)])), (*b"CPAL", cpal(&[RED]))]);
    let px = pixels(&layout_with("H", font.clone(), Color::rgba(0.0, 0.0, 0.0, 0.5)), PixelFormat::RGBA8);
    let top = px.iter().map(|p| p[3]).fold(0.0, f32::max);
    assert!((top - 0.5).abs() < 0.01, "max alpha {top}");
    assert!(px.iter().filter(|p| p[3] > 0.1).all(|p| p[0] > 0.98 && p[1] < 0.02), "red at half opacity");
    // Grayscale documents get the red's grey.
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U16, true);
    let px = pixels(&layout_with("H", font, Color::BLACK), gray);
    let solid: Vec<_> = px.iter().filter(|p| p[3] > 0.99).collect();
    assert!(!solid.is_empty() && solid.iter().all(|p| p[0] > 0.1 && p[0] < 0.6), "{:?}", solid.first());
}

#[test]
fn colour_glyph_ink_covers_its_layers() {
    let inter = fonts::INTER_REGULAR.as_slice();
    let (i, w) = (gid(inter, 'I'), gid(inter, 'W'));
    // A narrow I drawn as a wide W: the layer's ink is the glyph's ink.
    let font = with_tables(inter, &[(*b"COLR", colr_v0(i, &[(w, 0)])), (*b"CPAL", cpal(&[RED]))]);
    let plain = ink_rect(&layout_with("I", inter.to_vec(), Color::BLACK), &Affine::IDENTITY);
    let colour = ink_rect(&layout_with("I", font.clone(), Color::BLACK), &Affine::IDENTITY);
    assert!(colour.width() > plain.width() * 3, "{plain:?} vs {colour:?}");
    let px = pixels(&layout_with("I", font, Color::BLACK), PixelFormat::RGBA8);
    assert!(px.iter().filter(|p| p[3] > 0.99).count() > 400);
}

#[test]
fn colr_v1_gradient_runs_across_the_glyph() {
    let inter = fonts::INTER_REGULAR.as_slice();
    let h = gid(inter, 'H');
    let upem =
        FontRef::new(inter).map(|f| f.metrics(skrifa::instance::Size::unscaled(), skrifa::instance::LocationRef::default()).units_per_em).unwrap_or(2048);
    let font = with_tables(inter, &[(*b"COLR", colr_v1_gradient(h, 0, upem as i16 * 3 / 4)), (*b"CPAL", cpal(&[RED, BLUE]))]);
    for format in depths() {
        let l = layout_with("H", font.clone(), Color::BLACK);
        let r = rasterize(&l, &Affine::IDENTITY, format, AntiAlias::Sharp);
        let w = r.rect.width() as usize;
        let data = r.surface.read_region(r.rect);
        let px: Vec<[f32; 4]> = data.as_chunks::<4>().0.to_vec();
        // The leftmost opaque column is mostly red, the rightmost mostly blue, with no green.
        let opaque: Vec<(usize, [f32; 4])> = px.iter().enumerate().filter(|(_, p)| p[3] > 0.99).map(|(i, p)| (i % w, *p)).collect();
        let (lo, hi) = (opaque.iter().map(|o| o.0).min().unwrap_or(0), opaque.iter().map(|o| o.0).max().unwrap_or(0));
        assert!(hi > lo + 10, "{format:?}: drawn {lo}..{hi}");
        for (x, p) in &opaque {
            assert!(p[1] < 0.02, "{format:?}: green at {x}");
            if *x == lo {
                assert!(p[0] > 0.8 && p[2] < 0.2, "{format:?}: left {p:?}");
            }
            if *x == hi {
                assert!(p[2] > 0.8 && p[0] < 0.2, "{format:?}: right {p:?}");
            }
        }
    }
}

/// A COLR v1 table whose `base` paints itself (PaintColrGlyph), a cycle skrifa refuses to walk.
fn colr_v1_cycle(base: u16) -> Vec<u8> {
    let mut t = Vec::new();
    for v in [1u16, 0] {
        t.extend_from_slice(&v.to_be_bytes());
    }
    t.extend_from_slice(&0u32.to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&34u32.to_be_bytes());
    for _ in 0..4 {
        t.extend_from_slice(&0u32.to_be_bytes());
    }
    t.extend_from_slice(&1u32.to_be_bytes());
    t.extend_from_slice(&base.to_be_bytes());
    t.extend_from_slice(&10u32.to_be_bytes());
    // PaintColrGlyph (format 11) of itself.
    t.push(11);
    t.extend_from_slice(&base.to_be_bytes());
    t
}

#[test]
fn a_colour_glyph_skrifa_cannot_walk_draws_its_outline() {
    // A paint graph that loops back on itself fails in skrifa; the glyph keeps its plain
    // outline in the text colour instead of vanishing.
    let inter = fonts::INTER_REGULAR.as_slice();
    let h = gid(inter, 'H');
    let font = with_tables(inter, &[(*b"COLR", colr_v1_cycle(h)), (*b"CPAL", cpal(&[RED]))]);
    let green = Color::rgb(0.0, 1.0, 0.0);
    let px = pixels(&layout_with("H", font, green), PixelFormat::RGBA8);
    let solid: Vec<_> = px.iter().filter(|p| p[3] > 0.99).collect();
    assert!(solid.len() > 100, "{} pixels", solid.len());
    assert!(solid.iter().all(|p| p[1] > 0.98 && p[0] < 0.02), "{:?}", solid.first());
}

#[test]
fn a_colour_glyph_without_a_base_outline_still_paints() {
    // The base glyph has no outline of its own (no glyf entry); its colour layer does.
    let inter = fonts::INTER_REGULAR.as_slice();
    let h = gid(inter, 'H');
    let base = 60000u16;
    let font = with_tables(inter, &[(*b"COLR", colr_v0(base, &[(h, 0)])), (*b"CPAL", cpal(&[RED]))]);
    assert!(FontRef::new(&font).is_ok_and(|f| f.outline_glyphs().get(skrifa::GlyphId::new(base.into())).is_none()));
    let mut l = layout_with("H", font, Color::BLACK);
    for g in &mut l.glyphs {
        g.id = base.into();
    }
    let px = pixels(&l, PixelFormat::RGBA8);
    let red = px.iter().filter(|p| p[3] > 0.99 && p[0] > 0.98 && p[1] < 0.02).count();
    assert!(red > 100, "{red} red pixels");
}

#[test]
fn colour_glyphs_keep_extended_text_colours_in_float_documents() {
    // A layer in the text colour (palette entry 0xFFFF) keeps HDR and signed components in a
    // float document, as plain glyphs do, and stays bounded in integer ones.
    let inter = fonts::INTER_REGULAR.as_slice();
    let h = gid(inter, 'H');
    let font = with_tables(inter, &[(*b"COLR", colr_v0(h, &[(h, 0xFFFF)])), (*b"CPAL", cpal(&[RED]))]);
    let color = Color::rgba(2.5, -0.25, 0.5, 1.0);
    let float = pixels(&layout_with("H", font.clone(), color), PixelFormat::RGBA32F);
    let covered: Vec<_> = float.iter().filter(|p| p[3] > 0.01).collect();
    assert!(covered.len() > 100);
    assert!(covered.iter().all(|p| (p[0] - 2.5).abs() < 1e-4 && (p[1] + 0.25).abs() < 1e-4), "{:?}", covered.first());
    let int = pixels(&layout_with("H", font, color), PixelFormat::RGBA8);
    assert!(int.iter().filter(|p| p[3] > 0.01).all(|p| p[0] == 1.0 && p[1] == 0.0));
}

/// A COLR v1 table whose `base` paints `paint` (format bytes, offsets relative to each table).
fn colr_v1(base: u16, paint: &[u8]) -> Vec<u8> {
    let mut t = Vec::new();
    for v in [1u16, 0] {
        t.extend_from_slice(&v.to_be_bytes());
    }
    t.extend_from_slice(&0u32.to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&34u32.to_be_bytes());
    for _ in 0..4 {
        t.extend_from_slice(&0u32.to_be_bytes());
    }
    t.extend_from_slice(&1u32.to_be_bytes());
    t.extend_from_slice(&base.to_be_bytes());
    t.extend_from_slice(&10u32.to_be_bytes());
    t.extend_from_slice(paint);
    t
}

/// `depth` PaintComposites, each with the next one as both its source and its backdrop, over
/// `base` filled with palette entry `entry`: 2^depth fills from a few dozen bytes.
fn fan_out(base: u16, depth: usize, entry: u16) -> Vec<u8> {
    let mut p = Vec::new();
    for _ in 0..depth {
        // PaintComposite (format 32): source 8 bytes in, SrcOver, backdrop 8 bytes in.
        p.extend_from_slice(&[32, 0, 0, 8, 3, 0, 0, 8]);
    }
    // PaintGlyph (format 10) of `base`, then PaintSolid (format 2), alpha 1.
    p.extend_from_slice(&[10, 0, 0, 6]);
    p.extend_from_slice(&base.to_be_bytes());
    p.push(2);
    p.extend_from_slice(&entry.to_be_bytes());
    p.extend_from_slice(&(1i16 << 14).to_be_bytes());
    p
}

#[test]
fn a_colour_glyph_that_fans_out_draws_its_outline() {
    // A small graph that reuses both children of every composite would be walked 65536 times
    // over (for the bounds, the ink and the paint); skrifa stops after a fixed number of paints
    // and the glyph keeps its plain outline, as with a cycle.
    let inter = fonts::INTER_REGULAR.as_slice();
    let h = gid(inter, 'H');
    let font = with_tables(inter, &[(*b"COLR", colr_v1(h, &fan_out(h, 16, 0))), (*b"CPAL", cpal(&[RED]))]);
    let green = Color::rgb(0.0, 1.0, 0.0);
    let px = pixels(&layout_with("H", font, green), PixelFormat::RGBA8);
    let solid: Vec<_> = px.iter().filter(|p| p[3] > 0.99).collect();
    assert!(solid.len() > 100, "{} pixels", solid.len());
    assert!(solid.iter().all(|p| p[1] > 0.98 && p[0] < 0.02), "{:?}", solid.first());
}

#[test]
fn a_hidden_colour_glyph_does_not_spend_the_visible_texts_budget() {
    // A huge colour H at opacity 0 whose paint alone would overrun the pixel budget, then a
    // plain I in green: the H paints nothing anyway, and the I still shows.
    let inter = fonts::INTER_REGULAR.as_slice();
    let h = gid(inter, 'H');
    let font = with_tables(inter, &[(*b"COLR", colr_v1(h, &fan_out(h, 11, 0))), (*b"CPAL", cpal(&[RED]))]);
    let style = |color| CharStyle { size_pt: 600.0, color, ..Default::default() };
    let runs = vec![TextRun { len: 1, style: style(Color::TRANSPARENT) }, TextRun { len: 1, style: style(Color::rgb(0.0, 1.0, 0.0)) }];
    let layer = TextLayer { text: "HI".into(), font_family: "Inter".into(), runs, ..Default::default() };
    let mut l = TextEngine::new().layout(&layer, 72.0);
    for f in &mut l.faces {
        f.font = parley::FontData::new(parley::fontique::Blob::new(Arc::new(font.clone())), 0);
    }
    let px = pixels(&l, PixelFormat::RGBA8);
    let green = px.iter().filter(|p| p[3] > 0.99 && p[1] > 0.98 && p[0] < 0.02).count();
    assert!(green > 1000, "{green} green pixels");
    assert!(px.iter().all(|p| p[3] < 0.01 || p[0] < 0.02), "the hidden H showed");
}

#[test]
fn colour_glyph_layers_in_the_text_colour_keep_its_cmyk_components() {
    // In a CMYK document a K-only text colour stays K only in a 0xFFFF layer, as in plain text,
    // instead of coming back from sRGB as a four-colour black.
    let inter = fonts::INTER_REGULAR.as_slice();
    let h = gid(inter, 'H');
    let font = with_tables(inter, &[(*b"COLR", colr_v0(h, &[(h, 0xFFFF)])), (*b"CPAL", cpal(&[RED]))]);
    let k = Color { mode: ColorMode::Cmyk, c: [0.0, 0.0, 0.0, 1.0], alpha: 1.0 };
    let r = rasterize(&layout_with("H", font, k), &Affine::IDENTITY, PixelFormat::CMYKA8, AntiAlias::Sharp);
    let data = r.surface.read_region(r.rect);
    let solid: Vec<&[f32; 5]> = data.as_chunks::<5>().0.iter().filter(|p| p[4] > 0.99).collect();
    assert!(solid.len() > 100, "{} pixels", solid.len());
    assert!(solid.iter().all(|p| p[0] < 0.01 && p[1] < 0.01 && p[2] < 0.01 && p[3] > 0.99), "{:?}", solid.first());
}
