//! Advanced Blending (knockout, blend clipped / interior as group, transparency shapes, masks
//! hide effects) on small synthetic documents with hand-computed pixels, at 8, 16 and 32 bits.

use super::*;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Effect, FxCommon, FxPaint, Knockout, LayerMask, StrokeFx, StrokePosition};
use photocraft_geom::Size;

const E: f32 = 2.0 / 255.0;
const DEPTHS: [SampleType; 3] = [SampleType::U8, SampleType::U16, SampleType::F32];

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];
const GRAY: [f32; 4] = [0.5, 0.5, 0.5, 1.0];

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() <= E)
}

#[track_caller]
fn check(d: &Document, x: i32, y: i32, want: [f32; 4], what: &str) {
    let got = render(d, Rect::from_xywh(x, y, 1, 1)).px[0];
    assert!(close(got, want), "{what} ({:?}) at ({x},{y}): got {got:?}, want {want:?}", d.depth);
    // Tiled and whole-canvas renders agree (knockout targets are per tile).
    let tiled = render_tiled(d, d.bounds(), 3);
    let whole = render(d, d.bounds());
    assert!(tiled.px.iter().zip(&whole.px).all(|(a, b)| close(*a, *b)), "{what}: tiles differ from the whole render");
}

/// A 12×8 document at `depth` with a white Background (or none).
fn doc(depth: SampleType, background: bool) -> Document {
    let size = Size::new(12, 8);
    if background { Document::with_background("t", size, ColorMode::Rgb, depth, Color::WHITE) } else { Document::new("t", size, ColorMode::Rgb, depth) }
}

fn solid(d: &Document, name: &str, r: Rect, c: [f32; 4]) -> Layer {
    let mut l = Layer::raster(name, d.pixel_format());
    l.surface_mut().unwrap().fill_rect(r, &photocraft_raster::from_rgba(&d.pixel_format(), c));
    l
}

fn full(d: &Document) -> Rect {
    d.bounds()
}

/// The knocking-out layer: blue over the left half (x < 6), fill 0 unless set.
fn knocker(d: &Document, k: Knockout) -> Layer {
    let mut l = solid(d, "knock", Rect::new(0, 0, 6, 8), BLUE);
    l.advanced.knockout = k;
    l.fill_opacity = 0.0;
    l
}

#[test]
fn shallow_knockout_stops_at_the_bottom_of_an_isolated_group() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let green = solid(&d, "green", full(&d), GREEN);
        let red = solid(&d, "red", full(&d), RED);
        let mut g = Layer::group("g", vec![red, knocker(&d, Knockout::Shallow)]);
        g.blend = BlendMode::Normal;
        d.layers.push(green);
        d.layers.push(g);
        // The group's own buffer starts transparent: the layers beneath the group show.
        check(&d, 2, 2, GREEN, "shallow, fill 0");
        check(&d, 9, 2, RED, "outside the shape");
        d.layers[2].children_mut().unwrap()[1].fill_opacity = 0.5;
        // Blue at 50 % over the knocked-out (transparent) area, then the group over green.
        check(&d, 2, 2, [0.0, 0.5, 0.5, 1.0], "shallow, fill 50 %");
        // No knockout: blue at 50 % over red.
        d.layers[2].children_mut().unwrap()[1].advanced.knockout = Knockout::None;
        check(&d, 2, 2, [0.5, 0.0, 0.5, 1.0], "no knockout");
    }
}

#[test]
fn deep_knockout_reaches_the_background_through_groups() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let green = solid(&d, "green", full(&d), GREEN);
        let red = solid(&d, "red", full(&d), RED);
        // Pass-through groups let a deep knockout through to the Background.
        let inner = Layer::group("inner", vec![knocker(&d, Knockout::Deep)]);
        let outer = Layer::group("outer", vec![red, inner]);
        d.layers.push(green);
        d.layers.push(outer);
        check(&d, 2, 2, WHITE, "deep reveals the Background");
        check(&d, 9, 2, RED, "outside the shape");
        // An isolated group stops it at its own (transparent) bottom, like a shallow knockout
        // (psd-tools knockout-deep-nested): the layers beneath the group show.
        d.layers[2].blend = BlendMode::Normal;
        check(&d, 2, 2, GREEN, "deep inside an isolated group");
        d.layers[2].blend = BlendMode::PassThrough;
        d.layers[2].children_mut().unwrap()[1].blend = BlendMode::Normal;
        // The knockout is the inner group's only layer: its buffer stays transparent, red shows.
        check(&d, 2, 2, RED, "deep inside an isolated inner group");
    }
}

#[test]
fn top_level_shallow_knockout_reaches_the_background_or_transparency() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let green = solid(&d, "green", full(&d), GREEN);
        d.layers.push(green);
        d.layers.push(knocker(&d, Knockout::Shallow));
        check(&d, 2, 2, WHITE, "top-level shallow");
        // Layer opacity sets the knockout's strength.
        d.layers[2].opacity = 0.5;
        check(&d, 2, 2, [0.5, 1.0, 0.5, 1.0], "half-strength knockout");
        // Without a Background a deep knockout reveals transparency.
        let mut nobg = doc(depth, false);
        let green = solid(&nobg, "green", full(&nobg), GREEN);
        nobg.layers.push(green);
        nobg.layers.push(knocker(&nobg, Knockout::Deep));
        check(&nobg, 2, 2, [0.0; 4], "deep without a Background");
        check(&nobg, 9, 2, GREEN, "outside the shape");
        // A hidden Background doesn't show through either.
        d.layers[0].visible = false;
        d.layers[2].opacity = 1.0;
        check(&d, 2, 2, [0.0; 4], "hidden Background");
    }
}

#[test]
fn shallow_knockout_in_a_pass_through_group_reveals_what_is_beneath_it() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let green = solid(&d, "green", full(&d), GREEN);
        let red = solid(&d, "red", full(&d), RED);
        let g = Layer::group("pt", vec![red, knocker(&d, Knockout::Shallow)]);
        assert_eq!(g.blend, BlendMode::PassThrough);
        d.layers.push(green);
        d.layers.push(g);
        check(&d, 2, 2, GREEN, "pass-through shallow");
        d.layers[2].children_mut().unwrap()[1].advanced.knockout = Knockout::Deep;
        check(&d, 2, 2, WHITE, "pass-through deep");
    }
}

#[test]
fn knockout_on_a_group_uses_its_composite_shape_and_fill() {
    // psd-tools knockout-deep-normal: a group with Deep knockout at 50 % fill.
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let green = solid(&d, "green", full(&d), GREEN);
        let blue = solid(&d, "blue", Rect::new(0, 0, 6, 8), BLUE);
        let mut g = Layer::group("g", vec![blue]);
        g.blend = BlendMode::Normal;
        g.fill_opacity = 0.5;
        g.advanced.knockout = Knockout::Deep;
        d.layers.push(green);
        d.layers.push(g);
        check(&d, 2, 2, [0.5, 0.5, 1.0, 1.0], "blue at 50 % over the Background");
        check(&d, 9, 2, GREEN, "outside the group's pixels");
    }
}

#[test]
fn knockout_follows_masks_and_transparency_shapes() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let green = solid(&d, "green", full(&d), GREEN);
        d.layers.push(green);
        let mut k = knocker(&d, Knockout::Deep);
        // A mask hiding x >= 3: only x < 3 knocks out.
        let mut m = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
        m.fill_rect(Rect::new(0, 0, 3, 8), &photocraft_raster::from_rgba(&m.format(), [1.0; 4]));
        k.mask = Some(LayerMask { surface: m, enabled: true, linked: true, density: 1.0, feather: 0.0 });
        d.layers.push(k);
        check(&d, 1, 2, WHITE, "inside the mask");
        check(&d, 4, 2, GREEN, "masked out");
        // Transparency Shapes Layer off: the knockout covers the whole layer (masks still apply).
        d.layers[2].mask = None;
        d.layers[2].advanced.transparency_shapes = false;
        check(&d, 9, 2, WHITE, "beyond the pixels, tsly off");
    }
}

#[test]
fn clipped_layer_shallow_knockout_stops_at_the_base() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let base = solid(&d, "base", full(&d), RED);
        let mut mid = solid(&d, "mid", full(&d), GREEN);
        mid.clipped = true;
        let mut k = knocker(&d, Knockout::Shallow);
        k.clipped = true;
        d.layers.push(base);
        d.layers.push(mid);
        d.layers.push(k);
        check(&d, 2, 2, RED, "the base shows through the clipped layer above it");
        check(&d, 9, 2, GREEN, "outside");
        d.layers[3].advanced.knockout = Knockout::Deep;
        check(&d, 2, 2, RED, "clipped deep stops at the base too");
    }
}

#[test]
fn blend_clipped_layers_individually() {
    for depth in DEPTHS {
        let mut d = doc(depth, false);
        let gray = solid(&d, "gray", full(&d), GRAY);
        let mut base = solid(&d, "base", Rect::new(0, 0, 6, 8), RED);
        base.blend = BlendMode::Screen;
        let mut clip = solid(&d, "clip", full(&d), BLUE);
        clip.clipped = true;
        clip.blend = BlendMode::Multiply;
        d.layers.push(gray);
        d.layers.push(base);
        d.layers.push(clip);
        // As a group: red × blue = black, screened onto gray = gray.
        check(&d, 2, 2, GRAY, "as group");
        // One by one: red screened onto gray = (1, .5, .5), then × blue = (0, 0, .5).
        d.layers[1].advanced.blend_clipped = false;
        check(&d, 2, 2, [0.0, 0.0, 0.5, 1.0], "individually");
        check(&d, 9, 2, GRAY, "outside the base");
        // The base's opacity limits the clipped layers too.
        d.layers[1].opacity = 0.5;
        // Base at 50 %: (.75, .5, .5); the multiply then mixes in at 50 %: (.375, .25, .5).
        check(&d, 2, 2, [0.375, 0.25, 0.5, 1.0], "half-opacity base");
    }
}

fn overlay(c: [f32; 3]) -> Effect {
    Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 1.0), color: Color::rgb(c[0], c[1], c[2]) }
}

#[test]
fn blend_interior_effects_as_group_puts_overlays_under_fill_opacity() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let mut l = solid(&d, "l", Rect::new(0, 0, 6, 8), BLUE);
        l.effects.items = vec![overlay([1.0, 0.0, 0.0])];
        l.fill_opacity = 0.0;
        d.layers.push(l);
        check(&d, 2, 2, RED, "default: overlays ignore fill");
        d.layers[1].advanced.blend_interior = true;
        check(&d, 2, 2, WHITE, "as group at fill 0");
        d.layers[1].fill_opacity = 0.5;
        check(&d, 2, 2, [1.0, 0.5, 0.5, 1.0], "as group at fill 50 %");
        d.layers[1].advanced.blend_interior = false;
        check(&d, 2, 2, RED, "default at fill 50 %");
    }
}

#[test]
fn interior_effects_blend_over_the_layer_mode_unless_grouped() {
    // #3996: #888888 under a black Color-mode layer with a red Overlay colour overlay. The
    // layer's mode applies to its own pixels (grey stays grey), then the overlay blends onto
    // that result and tints it; as a group the overlay of black is black, so grey remains.
    let grey = 136.0 / 255.0;
    let lift = 1.0 - 2.0 * (1.0 - grey);
    for depth in DEPTHS {
        let mut d = doc(depth, false);
        d.layers.push(solid(&d, "grey", full(&d), [grey, grey, grey, 1.0]));
        let mut l = solid(&d, "black", full(&d), [0.0, 0.0, 0.0, 1.0]);
        l.blend = BlendMode::Color;
        l.effects.items = vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::Overlay, 1.0), color: Color::rgb(1.0, 0.0, 0.0) }];
        d.layers.push(l);
        check(&d, 2, 2, [1.0, lift, lift, 1.0], "default: overlay over the Color blend");
        d.layers[1].advanced.blend_interior = true;
        check(&d, 2, 2, [grey, grey, grey, 1.0], "as group: overlay inside the Color blend");
    }
}

#[test]
fn interior_effects_blend_with_the_backdrop_through_a_see_through_normal_layer() {
    // #6232: #04170F under a Normal layer at fill 0 with a yellow Soft Light colour overlay. The
    // overlay soft-lights the dark green it sees through the layer (Photoshop's soft light of
    // yellow: the W3C D(b) for red and green, which is the cubic ((16b − 12)b + 4)b below a
    // quarter (#8804), and b² for blue), not transparency, which would leave plain yellow. A
    // half-transparent part of the layer mixes it in at half strength.
    let bg: [f32; 3] = [4.0 / 255.0, 23.0 / 255.0, 15.0 / 255.0];
    let dark = |b: f32| ((16.0 * b - 12.0) * b + 4.0) * b;
    let lit = [dark(bg[0]), dark(bg[1]), bg[2] * bg[2]];
    let half = |c: usize| (bg[c] + lit[c]) / 2.0;
    for depth in DEPTHS {
        let mut d = doc(depth, false);
        d.layers.push(solid(&d, "green", full(&d), [bg[0], bg[1], bg[2], 1.0]));
        let mut l = solid(&d, "fade", Rect::new(0, 0, 6, 8), [0.2, 0.6, 0.9, 1.0]);
        l.surface_mut().unwrap().fill_rect(Rect::new(6, 0, 12, 8), &photocraft_raster::from_rgba(&d.pixel_format(), [0.2, 0.6, 0.9, 0.5]));
        l.fill_opacity = 0.0;
        l.effects.items = vec![Effect::ColorOverlay { common: FxCommon::new(BlendMode::SoftLight, 1.0), color: Color::rgb(1.0, 1.0, 0.0) }];
        d.layers.push(l);
        check(&d, 2, 2, [lit[0], lit[1], lit[2], 1.0], "opaque part");
        check(&d, 9, 2, [half(0), half(1), half(2), 1.0], "half-transparent part");
    }
}

#[test]
fn a_see_through_normal_layer_without_interior_effects_counts_the_backdrop_once() {
    // A half-transparent backdrop under a Normal layer at fill 50 % whose only effect is a drop
    // shadow (cast off the canvas): plain Normal compositing, blue at 50 % over red at 50 %. The
    // layer must not merge into the backdrop first, or the backdrop shows twice (alpha 0.875).
    for depth in DEPTHS {
        let mut d = doc(depth, false);
        d.layers.push(solid(&d, "red", full(&d), [1.0, 0.0, 0.0, 0.5]));
        let mut l = solid(&d, "blue", Rect::new(0, 0, 12, 4), BLUE);
        l.fill_opacity = 0.5;
        l.effects.items = vec![Effect::DropShadow(photocraft_doc::Shadow {
            common: FxCommon::new(BlendMode::Normal, 1.0),
            color: Color::BLACK,
            angle: 90.0,
            use_global_light: false,
            distance: 20.0,
            spread: 1.0,
            size: 0.0,
            contour: photocraft_doc::Contour::Linear,
            anti_alias: false,
            noise: 0.0,
            knocks_out: true,
        })];
        d.layers.push(l);
        check(&d, 2, 2, [1.0 / 3.0, 0.0, 2.0 / 3.0, 0.75], "inside the layer");
    }
}

#[test]
fn transparency_shapes_off_spreads_effects_over_the_whole_layer() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let mut l = solid(&d, "l", Rect::new(0, 0, 6, 8), BLUE);
        l.effects.items = vec![overlay([1.0, 0.0, 0.0])];
        d.layers.push(l);
        check(&d, 9, 2, WHITE, "default: the overlay follows the pixels");
        d.layers[1].advanced.transparency_shapes = false;
        check(&d, 9, 2, RED, "tsly off: the overlay covers the layer");
        check(&d, 2, 2, RED, "inside");
        // At fill 0 the content's transparency no longer matters.
        d.layers[1].fill_opacity = 0.0;
        check(&d, 9, 2, RED, "tsly off, fill 0");
    }
}

fn inside_stroke() -> Effect {
    Effect::Stroke(StrokeFx {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        size: 2.0,
        position: StrokePosition::Inside,
        paint: FxPaint::Color(Color::rgb(1.0, 0.0, 0.0)),
        overprint: false,
    })
}

fn outside_stroke() -> Effect {
    Effect::Stroke(StrokeFx {
        common: FxCommon::new(BlendMode::Normal, 1.0),
        size: 6.0,
        position: StrokePosition::Outside,
        paint: FxPaint::Color(Color::rgb(1.0, 0.0, 0.0)),
        overprint: false,
    })
}

#[test]
fn shapeless_stroke_tile_skips_match_full_region_across_blending_settings() {
    use photocraft_doc::BlendRange;

    let modes = [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Difference];
    for depth in DEPTHS {
        for position in [StrokePosition::Outside, StrokePosition::Inside, StrokePosition::Center] {
            for blend in modes {
                for fill in [0.0, 0.5, 1.0] {
                    for blend_if in [false, true] {
                        let mut d = Document::with_background("t", Size::new(80, 768), ColorMode::Rgb, depth, Color::WHITE);
                        let mut l = solid(&d, "stroke", Rect::new(28, 318, 52, 342), BLUE);
                        l.effects.items = vec![Effect::Stroke(StrokeFx {
                            common: FxCommon::new(BlendMode::Normal, 1.0),
                            size: 6.0,
                            position,
                            paint: FxPaint::Color(Color::rgb(1.0, 0.0, 0.0)),
                            overprint: false,
                        })];
                        l.blend = blend;
                        l.fill_opacity = fill;
                        l.advanced.transparency_shapes = false;
                        if blend_if {
                            l.blend_if.set(0, [BlendRange::FULL, BlendRange { black: [128, 128], white: [255, 255] }]);
                        }
                        d.layers.push(l);

                        let rect = Rect::new(0, 0, 80, 768);
                        let whole = render_tiled(&d, rect, 1024);
                        let tiled = render_tiled(&d, rect, 64);
                        assert_eq!(whole.px, tiled.px, "{depth:?}, {position:?}, {blend:?}, fill={fill}, blend_if={blend_if}");

                        let mut bands = Vec::new();
                        render_bands(&d, rect, 256, |band| -> Result<(), ()> {
                            bands.push(band);
                            Ok(())
                        })
                        .unwrap();
                        let mut band_pixels = Vec::with_capacity(whole.px.len());
                        for band in bands {
                            band_pixels.extend(band.px);
                        }
                        assert_eq!(whole.px, band_pixels, "banded: {depth:?}, {position:?}, {blend:?}, fill={fill}, blend_if={blend_if}");
                    }
                }
            }
        }
    }
}

#[test]
fn shapeless_stroke_skip_rejects_advanced_blending_and_masks() {
    use photocraft_doc::{BlendRange, Path, Subpath, VectorMask};

    let d = doc(SampleType::U8, true);
    let mut l = solid(&d, "stroke", Rect::new(2, 2, 8, 8), BLUE);
    l.effects.items = vec![outside_stroke()];
    l.advanced.transparency_shapes = false;
    assert!(shapeless_stroke_bounds(&l).is_some());

    l.advanced.knockout = Knockout::Deep;
    assert!(shapeless_stroke_bounds(&l).is_none(), "knockout changes the backdrop outside the content");
    l.advanced.knockout = Knockout::None;

    l.advanced.blend_interior = true;
    assert!(shapeless_stroke_bounds(&l).is_none(), "Blend Interior effects remain on the uncropped path");
    l.advanced.blend_interior = false;

    l.blend_if.set(0, [BlendRange::FULL, BlendRange { black: [128, 128], white: [255, 255] }]);
    assert!(shapeless_stroke_bounds(&l).is_none(), "Blend If remains on the uncropped path");
    l.blend_if = Default::default();

    l.mask = Some(LayerMask::hide_all());
    assert!(shapeless_stroke_bounds(&l).is_none(), "enabled masks remain on the uncropped path");
    l.mask = None;

    l.vector_mask = Some(VectorMask::new(Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)])])));
    assert!(shapeless_stroke_bounds(&l).is_none(), "enabled vector masks remain on the uncropped path");
}

#[test]
fn layer_mask_hides_effects() {
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let mut l = solid(&d, "l", full(&d), BLUE);
        l.effects.items = vec![inside_stroke()];
        // The mask shows x < 8.
        let mut m = photocraft_raster::Surface::new(PixelFormat::new(ColorMode::Grayscale, depth, false));
        m.fill_rect(Rect::new(0, 0, 8, 8), &photocraft_raster::from_rgba(&m.format(), [1.0; 4]));
        l.mask = Some(LayerMask { surface: m, enabled: true, linked: true, density: 1.0, feather: 0.0 });
        d.layers.push(l);
        // Default: the stroke follows the masked shape, so it runs along the mask's edge.
        check(&d, 7, 4, RED, "stroke along the mask edge");
        check(&d, 9, 4, WHITE, "masked out");
        check(&d, 4, 4, BLUE, "interior");
        // Layer Mask Hides Effects: the stroke is built from the whole layer (canvas edges only)
        // and the mask then hides the result.
        d.layers[1].advanced.layer_mask_hides_effects = true;
        check(&d, 7, 4, BLUE, "no stroke at the mask edge");
        check(&d, 4, 0, RED, "the canvas-edge stroke stays");
        check(&d, 9, 4, WHITE, "still masked out");
        check(&d, 11, 0, WHITE, "the stroke beyond the mask is hidden");
        // The vector-mask switch alone changes nothing for a pixel mask.
        d.layers[1].advanced.layer_mask_hides_effects = false;
        d.layers[1].advanced.vector_mask_hides_effects = true;
        check(&d, 7, 4, RED, "vmgm without a vector mask");
    }
}

#[test]
fn vector_mask_hides_effects() {
    use photocraft_doc::{Path, Subpath, VectorMask};
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        let mut l = solid(&d, "l", full(&d), BLUE);
        l.effects.items = vec![inside_stroke()];
        let path = Path::new(vec![Subpath::polygon(&[(-4.0, -4.0), (8.0, -4.0), (8.0, 12.0), (-4.0, 12.0)])]);
        l.vector_mask = Some(VectorMask::new(path));
        d.layers.push(l);
        check(&d, 7, 4, RED, "stroke along the vector mask");
        d.layers[1].advanced.vector_mask_hides_effects = true;
        check(&d, 7, 4, BLUE, "vector mask hides the stroke");
        check(&d, 9, 4, WHITE, "masked out");
    }
}

/// The per-render vector-mask cache must key masks by content: `hiding_mask` and
/// `effects_source` hand it temporary clones whose addresses get reused (#4426).
#[test]
fn vector_mask_cache_does_not_mix_up_layers() {
    use photocraft_doc::{Path, Subpath, VectorMask};
    for depth in DEPTHS {
        let mut d = doc(depth, true);
        for (name, color, x0, x1) in [("a", BLUE, -4.0, 6.0), ("b", GREEN, 6.0, 16.0)] {
            let mut l = solid(&d, name, full(&d), color);
            l.effects.items = vec![inside_stroke()];
            l.advanced.vector_mask_hides_effects = true;
            let path = Path::new(vec![Subpath::polygon(&[(x0, -4.0), (x1, -4.0), (x1, 12.0), (x0, 12.0)])]);
            l.vector_mask = Some(VectorMask::new(path));
            d.layers.push(l);
        }
        check(&d, 2, 4, BLUE, "layer A inside its own mask");
        check(&d, 9, 4, GREEN, "layer B inside its own mask");
    }
}

#[test]
fn defaults_take_the_plain_paths() {
    let d = doc(SampleType::U8, true);
    let mut l = solid(&d, "l", full(&d), BLUE);
    assert!(!advanced_active(&l, &[]));
    l.advanced.knockout = Knockout::Shallow;
    assert!(advanced_active(&l, &[]));
    l.advanced.knockout = Knockout::None;
    // Switches that change nothing for this layer (no effects, no clipped layers) stay inactive.
    l.advanced.blend_interior = true;
    l.advanced.layer_mask_hides_effects = true;
    l.advanced.blend_clipped = false;
    assert!(!advanced_active(&l, &[]));
    let mut c = solid(&d, "c", full(&d), RED);
    c.clipped = true;
    assert!(advanced_active(&l, std::slice::from_ref(&c)));
    c.visible = false;
    assert!(!advanced_active(&l, std::slice::from_ref(&c)));
    l.effects.items = vec![overlay([1.0, 0.0, 0.0])];
    assert!(advanced_active(&l, &[]), "blend interior with effects");
}

#[test]
fn a_knockout_background_does_not_recurse() {
    // A Background carrying a knockout (only reachable through a file) renders without looping.
    let mut d = doc(SampleType::U8, true);
    d.layers[0].advanced.knockout = Knockout::Deep;
    d.layers[0].fill_opacity = 0.0;
    let got = render(&d, Rect::from_xywh(0, 0, 1, 1)).px[0];
    assert!(got[3] <= 1.0);
}

/// The Background is found by its structure, not its English name: a file from a German or
/// Japanese Photoshop calls it "Hintergrund" or "背景", and a deep knockout still reveals it. A
/// bottom layer merely called "Background", without the Background's locks, is not one.
#[test]
fn deep_knockout_reveals_a_localized_background() {
    for depth in DEPTHS {
        for name in ["Hintergrund", "背景", "Arrière-plan"] {
            let mut d = doc(depth, true);
            d.layers[0].name = name.into();
            let green = solid(&d, "green", full(&d), GREEN);
            d.layers.push(green);
            d.layers.push(knocker(&d, Knockout::Deep));
            check(&d, 2, 2, WHITE, name);
        }
        let mut d = doc(depth, true);
        d.layers[0].locks = Default::default();
        let green = solid(&d, "green", full(&d), GREEN);
        d.layers.push(green);
        d.layers.push(knocker(&d, Knockout::Deep));
        check(&d, 2, 2, [0.0; 4], "an unlocked layer named Background");
    }
}

/// Transparency Shapes Layer off in the special eight: the layer's own transparency blends as
/// Fill does (#5692). Measured in Photoshop: Background (90, 110, 130), a layer (200, 150, 80) at
/// alpha 30 % (77) and 70 % (179) in columns 0–5 and 6–11, read from the flattened 8-bit PNG.
#[test]
fn transparency_shapes_off_blends_alpha_as_fill() {
    const A30: f32 = 77.0 / 255.0;
    const A70: f32 = 179.0 / 255.0;
    let c8 = |c: [u8; 3]| [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0];
    let px = |c: [u8; 3], a: f32| {
        let c = c8(c);
        [c[0], c[1], c[2], a]
    };
    let make = |depth: SampleType, mode: BlendMode, opacity: f32, fill: f32, tsl: bool| {
        let b = c8([90, 110, 130]);
        let mut d = Document::with_background("t", Size::new(12, 8), ColorMode::Rgb, depth, Color::rgb(b[0], b[1], b[2]));
        let mut l = solid(&d, "a30", Rect::new(0, 0, 6, 8), px([200, 150, 80], A30));
        l.surface_mut().unwrap().fill_rect(Rect::new(6, 0, 12, 8), &photocraft_raster::from_rgba(&d.pixel_format(), px([200, 150, 80], A70)));
        l.blend = mode;
        l.opacity = opacity;
        l.fill_opacity = fill;
        l.advanced.transparency_shapes = tsl;
        d.layers.push(l);
        d
    };
    let (o53, f50) = (135.0 / 255.0, 128.0 / 255.0);
    // (mode, opacity, fill, off at 30 %, off at 70 %, on at 30 %, on at 70 %)
    type Rgb = [u8; 3];
    let measured: [(BlendMode, f32, f32, Rgb, Rgb, Rgb, Rgb); 7] = [
        (BlendMode::ColorDodge, o53, 1.0, [105, 123, 137], [148, 151, 150], [117, 133, 139], [151, 164, 152]),
        (BlendMode::ColorBurn, o53, 1.0, [84, 99, 113], [74, 79, 69], [83, 94, 109], [73, 72, 82]),
        (BlendMode::LinearDodge, o53, 1.0, [122, 134, 143], [164, 166, 160], [117, 133, 143], [151, 164, 160]),
        (BlendMode::HardMix, o53, 1.0, [98, 111, 119], [133, 117, 75], [117, 133, 109], [151, 164, 82]),
        (BlendMode::Difference, o53, 1.0, [58, 86, 117], [69, 54, 100], [93, 99, 117], [97, 84, 100]),
        (BlendMode::ColorDodge, 1.0, f50, [102, 121, 136], [125, 139, 146], [108, 124, 137], [131, 142, 147]),
        (BlendMode::Difference, 1.0, f50, [59, 87, 118], [19, 57, 102], [66, 87, 118], [34, 57, 102]),
    ];
    for depth in DEPTHS {
        for (mode, opacity, fill, off30, off70, on30, on70) in measured {
            // A 32-bit document's Linear Dodge doesn't clip at 1 (psblend::HDR).
            if depth == SampleType::F32 && mode == BlendMode::LinearDodge {
                continue;
            }
            let what = format!("{mode:?} opacity {opacity} fill {fill}");
            let off = make(depth, mode, opacity, fill, false);
            check(&off, 2, 4, px(off30, 1.0), &format!("{what}, off, alpha 30 %"));
            check(&off, 9, 4, px(off70, 1.0), &format!("{what}, off, alpha 70 %"));
            let on = make(depth, mode, opacity, fill, true);
            check(&on, 2, 4, px(on30, 1.0), &format!("{what}, on, alpha 30 %"));
            check(&on, 9, 4, px(on70, 1.0), &format!("{what}, on, alpha 70 %"));
            assert!(advanced_active(&off.layers[1], &[]), "{what}: the GPU leaves it to the CPU");
        }
        // Modes outside the special eight look the same either way, and stay on the plain path.
        for mode in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay] {
            let (off, on) = (make(depth, mode, o53, f50, false), make(depth, mode, o53, f50, true));
            assert_eq!(render(&off, off.bounds()).px, render(&on, on.bounds()).px, "{mode:?}");
            assert!(!advanced_active(&off.layers[1], &[]), "{mode:?}");
        }
    }
}

/// A mask is not the layer's transparency: Photoshop draws an opaque layer under a 30 % mask the
/// same with the option on or off (measured: Color Dodge, Hard Mix, Difference and the rest of
/// the special eight, every Opacity and Fill tried). Over a half-transparent backdrop and for an
/// isolated group the alpha still blends as Fill (measured: Color Dodge at 53 %).
#[test]
fn transparency_shapes_off_ignores_masks_and_reaches_groups() {
    let c8 = |c: [u8; 3]| [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0];
    let px = |c: [u8; 3], a: f32| {
        let c = c8(c);
        [c[0], c[1], c[2], a]
    };
    let o53 = 135.0 / 255.0;
    for depth in DEPTHS {
        for mode in [BlendMode::ColorDodge, BlendMode::HardMix, BlendMode::Difference] {
            let render_with = |tsl: bool| {
                let b = c8([90, 110, 130]);
                let mut d = Document::with_background("t", Size::new(12, 8), ColorMode::Rgb, depth, Color::rgb(b[0], b[1], b[2]));
                let mut l = solid(&d, "l", full(&d), px([200, 150, 80], 1.0));
                l.blend = mode;
                l.opacity = o53;
                l.mask = Some(LayerMask { surface: photocraft_raster::Surface::with_default(PixelFormat::GRAY8, &[77.0 / 255.0]), ..LayerMask::reveal_all() });
                l.advanced.transparency_shapes = tsl;
                d.layers.push(l);
                render(&d, d.bounds()).px
            };
            let (off, on) = (render_with(false), render_with(true));
            assert!(off.iter().zip(&on).all(|(a, b)| close(*a, *b)), "{mode:?} ({depth:?}): a mask isn't the layer's transparency");
        }
        // Color Dodge at 53 % over a backdrop of alpha 50 % (128): (118, 127, 129, 148) at
        // alpha 30 %, (162, 151, 131, 175) at 70 %.
        let mut d = doc(depth, false);
        d.layers.push(solid(&d, "under", full(&d), px([90, 110, 130], 128.0 / 255.0)));
        let mut l = solid(&d, "a30", Rect::new(0, 0, 6, 8), px([200, 150, 80], 77.0 / 255.0));
        l.surface_mut().unwrap().fill_rect(Rect::new(6, 0, 12, 8), &photocraft_raster::from_rgba(&d.pixel_format(), px([200, 150, 80], 179.0 / 255.0)));
        l.blend = BlendMode::ColorDodge;
        l.opacity = o53;
        l.advanced.transparency_shapes = false;
        d.layers.push(l.clone());
        check(&d, 2, 4, px([118, 127, 129], 148.0 / 255.0), "half-transparent backdrop, alpha 30 %");
        check(&d, 9, 4, px([162, 151, 131], 175.0 / 255.0), "half-transparent backdrop, alpha 70 %");
        // An isolated Color Dodge group at 53 % holding the Normal layer: the group's alpha blends
        // as Fill, as a layer's does.
        let b = c8([90, 110, 130]);
        let mut d = Document::with_background("t", Size::new(12, 8), ColorMode::Rgb, depth, Color::rgb(b[0], b[1], b[2]));
        l.blend = BlendMode::Normal;
        l.opacity = 1.0;
        l.advanced.transparency_shapes = true;
        let mut g = Layer::group("g", vec![l]);
        g.blend = BlendMode::ColorDodge;
        g.opacity = o53;
        g.advanced.transparency_shapes = false;
        d.layers.push(g);
        check(&d, 2, 4, px([105, 123, 137], 1.0), "group, alpha 30 %");
        check(&d, 9, 4, px([148, 151, 150], 1.0), "group, alpha 70 %");
    }
}

/// A clipped knockout in a Transparency Shapes Layer off base of the special eight: the knockout
/// stops at the base's own content (Blend Clipped Layers as Group), so it gives back the base's
/// colour without touching its alpha, and the base's transparency still blends as Fill under a
/// feathered vector mask. Where the fill-0 knocker covers an earlier clipped layer the composite
/// is the base's alone.
#[test]
fn transparency_shapes_off_keeps_the_base_alpha_under_a_clipped_knockout() {
    let square = photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polygon(&[(2.0, 1.0), (10.0, 1.0), (10.0, 7.0), (2.0, 7.0)])]);
    for depth in DEPTHS {
        for k in [Knockout::Shallow, Knockout::Deep] {
            let mut d = Document::with_background("t", Size::new(12, 8), ColorMode::Rgb, depth, Color::rgb(90.0 / 255.0, 110.0 / 255.0, 130.0 / 255.0));
            let mut base = solid(&d, "base", Rect::new(0, 0, 3, 8), [0.78, 0.59, 0.31, 0.3]);
            base.surface_mut().unwrap().fill_rect(Rect::new(3, 0, 12, 8), &photocraft_raster::from_rgba(&d.pixel_format(), [0.78, 0.59, 0.31, 0.7]));
            base.blend = BlendMode::ColorDodge;
            base.opacity = 0.53;
            base.advanced.transparency_shapes = false;
            base.vector_mask = Some(photocraft_doc::vector::VectorMask { feather: 1.5, ..photocraft_doc::vector::VectorMask::new(square.clone()) });
            d.layers.push(base);
            let alone = render(&d, d.bounds()).px;
            let mut red = solid(&d, "red", full(&d), [1.0, 0.0, 0.0, 0.5]);
            red.clipped = true;
            let mut knock = knocker(&d, k);
            knock.clipped = true;
            d.layers.extend([red, knock]);
            let got = render(&d, d.bounds()).px;
            let what = format!("{k:?} ({depth:?})");
            for (i, (g, a)) in got.iter().zip(&alone).enumerate() {
                if i % 12 < 6 {
                    assert!(close(*g, *a), "{what}: knocked out at {i}: {g:?}, the base alone {a:?}");
                }
            }
            assert!(!close(got[6 * 12 + 8], alone[6 * 12 + 8]), "{what}: the red shows outside the knocker");
            check(&d, 4, 3, alone[3 * 12 + 4], &what);
        }
    }
}
