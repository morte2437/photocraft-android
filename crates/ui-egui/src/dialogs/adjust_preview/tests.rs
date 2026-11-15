use super::*;
use photocraft_color::{BlendMode, Color, PixelFormat, SampleType};
use photocraft_doc::{BlendIf, BlendRange, Effect, FxCommon, Size};
use serde_json::json;

const KINDS: [&str; 14] = crate::adjust_editors::KINDS;

/// Settings that move every kind well away from neutral.
pub(crate) fn sample(kind: &str) -> Value {
    match kind {
        "brightnessContrast" => json!({"brightness": 40, "contrast": 30}),
        "levels" => json!({"inBlack": 30, "inWhite": 220, "gamma": 1.3, "outBlack": 10}),
        "curves" => json!({"points": [[0, 20], [90, 150], [255, 230]]}),
        "exposure" => json!({"exposure": 0.8, "offset": 0.02, "gamma": 0.9}),
        "vibrance" => json!({"vibrance": 50, "saturation": 20}),
        "hueSaturation" => json!({"hue": 25, "saturation": 30, "reds": {"hue": 40}}),
        "colorBalance" => json!({"midtones": [40, 0, -20], "shadows": [0, 20, 0]}),
        "blackWhite" => json!({"reds": 150, "tint": true}),
        "photoFilter" => json!({"filter": "cooling80", "density": 60}),
        "channelMixer" => json!({"red": [50, 50, 0, 0], "blue": [0, 20, 90, 5]}),
        "posterize" => json!({"levels": 3}),
        "threshold" => json!({"level": 90}),
        "gradientMap" => json!({"stops": [[0, "#200040"], [1, "#ffd080"]]}),
        "selectiveColor" => json!({"method": "absolute", "reds": [10, -20, 30, 5], "blues": [-15, 0, 25, 0]}),
        _ => json!({}),
    }
}

fn rnd(seed: u32, i: u32) -> f32 {
    let mut h = seed.wrapping_mul(0x9e37_79b9) ^ i.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    (h & 0xffff) as f32 / 65535.0
}

fn fill_noise(s: &mut photocraft_raster::Surface, r: Rect, seed: u32, min_alpha: f32) {
    let fmt = s.format();
    let ch = fmt.channels();
    let n = r.width() * r.height();
    let mut data = Vec::with_capacity(n as usize * ch);
    for i in 0..n {
        for c in 0..ch {
            let v = rnd(seed + c as u32 * 7919, i);
            let alpha = fmt.alpha && c == ch - 1;
            data.push(if alpha { min_alpha + (1.0 - min_alpha) * v } else { v });
        }
    }
    s.write_region(r, &data);
}

/// Background, the target (semi-transparent noise, 80 % Multiply, layer mask), a layer clipped to
/// the target and a Screen layer on top: what the preview layer is inserted into.
pub(crate) fn document(mode: ColorMode, depth: SampleType, selection: bool) -> (Document, LayerId) {
    let (w, h) = (40u32, 32u32);
    let mut doc = Document::with_background("t", Size::new(w, h), mode, depth, Color::rgba(0.7, 0.6, 0.5, 1.0));
    let fmt = doc.pixel_format();
    if let Some(s) = doc.layers[0].surface_mut() {
        fill_noise(s, Rect::from_xywh(0, 0, w, h / 2), 3, 1.0);
    }
    let mut target = Layer::raster("target", fmt);
    if let Some(s) = target.surface_mut() {
        fill_noise(s, Rect::from_xywh(4, 2, 30, 26), 11, 0.2);
    }
    target.opacity = 0.8;
    target.blend = BlendMode::Multiply;
    let mut m = LayerMask::reveal_all();
    m.surface.fill_rect(Rect::from_xywh(0, 0, 10, 10), &[0.3]);
    target.mask = Some(m);
    let id = target.id;
    let mut clip = Layer::raster("clipped", fmt);
    if let Some(s) = clip.surface_mut() {
        fill_noise(s, Rect::from_xywh(20, 10, 12, 12), 17, 0.5);
    }
    clip.clipped = true;
    clip.opacity = 0.6;
    let mut top = Layer::raster("top", fmt);
    if let Some(s) = top.surface_mut() {
        fill_noise(s, Rect::from_xywh(0, 20, 40, 12), 23, 0.0);
    }
    top.blend = BlendMode::Screen;
    doc.layers.extend([target, clip, top]);
    if selection {
        let mut sel = photocraft_raster::Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::from_xywh(8, 4, 24, 20), &[1.0]);
        sel.fill_rect(Rect::from_xywh(8, 4, 6, 20), &[0.5]);
        doc.selection = Some(sel);
    }
    (doc, id)
}

/// A target in the middle of a clipping group, nested two groups deep: a base, a clipped layer
/// below the target, the target (semi-transparent noise, 80 % Multiply, a mask) and a clipped
/// layer above it. The target is clipped, so its preview must isolate it from the group. It is
/// unstyled, which is the path the preview accelerates.
pub(crate) fn clipped_document(mode: ColorMode, depth: SampleType, selection: bool) -> (Document, LayerId) {
    let (w, h) = (40u32, 32u32);
    let mut doc = Document::with_background("t", Size::new(w, h), mode, depth, Color::rgba(0.7, 0.6, 0.5, 1.0));
    let fmt = doc.pixel_format();
    if let Some(s) = doc.layers[0].surface_mut() {
        fill_noise(s, Rect::from_xywh(0, 0, w, h / 2), 3, 1.0);
    }
    let mut base = Layer::raster("clip base", fmt);
    if let Some(s) = base.surface_mut() {
        fill_noise(s, Rect::from_xywh(0, 0, w, h), 5, 0.35);
    }
    base.opacity = 0.9;
    let mut below = Layer::raster("below", fmt);
    if let Some(s) = below.surface_mut() {
        fill_noise(s, Rect::from_xywh(2, 18, 30, 10), 7, 0.4);
    }
    below.clipped = true;
    below.opacity = 0.5;
    let mut target = Layer::raster("target", fmt);
    if let Some(s) = target.surface_mut() {
        fill_noise(s, Rect::from_xywh(4, 2, 30, 26), 11, 0.2);
    }
    target.clipped = true;
    target.opacity = 0.8;
    target.blend = BlendMode::Multiply;
    let mut m = LayerMask::reveal_all();
    m.surface.fill_rect(Rect::from_xywh(0, 0, 10, 10), &[0.3]);
    target.mask = Some(m);
    let id = target.id;
    let mut above = Layer::raster("above", fmt);
    if let Some(s) = above.surface_mut() {
        fill_noise(s, Rect::from_xywh(20, 10, 12, 12), 17, 0.5);
    }
    above.clipped = true;
    above.opacity = 0.6;
    let mut inner = Layer::group("inner", vec![base, below, target, above]);
    inner.opacity = 0.95;
    let outer = Layer::group("outer", vec![inner]);
    let mut top = Layer::raster("top", fmt);
    if let Some(s) = top.surface_mut() {
        fill_noise(s, Rect::from_xywh(0, 20, 40, 12), 23, 0.0);
    }
    top.blend = BlendMode::Screen;
    doc.layers.extend([outer, top]);
    if selection {
        let mut sel = photocraft_raster::Surface::new(PixelFormat::GRAY8);
        sel.fill_rect(Rect::from_xywh(8, 4, 24, 20), &[1.0]);
        sel.fill_rect(Rect::from_xywh(8, 4, 6, 20), &[0.5]);
        doc.selection = Some(sel);
    }
    (doc, id)
}

/// The document after running the real destructive command on `target`.
pub(crate) fn destructive(doc: &Document, target: LayerId, kind: &str, params: &Value) -> Option<Document> {
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc.clone(), None);
    s.select_layer(target).ok()?;
    s.execute(&format!("image.adjustments.{kind}"), params.clone()).ok()?;
    Some((*s.active()?.doc).clone())
}

/// Largest premultiplied difference between two composites.
pub(crate) fn max_diff(a: &Document, b: &Document) -> f32 {
    let (ra, rb) = (photocraft_compose::render(a, a.bounds()), photocraft_compose::render(b, b.bounds()));
    let pm = |p: &[f32; 4]| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]];
    ra.px.iter().zip(&rb.px).map(|(x, y)| (0..4).map(|c| (pm(x)[c] - pm(y)[c]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max)
}

/// Kinds that make colour from gray: grayscale documents preview them on the CPU proxy.
const GRAY_COLOUR: [&str; 7] = ["vibrance", "colorBalance", "blackWhite", "photoFilter", "channelMixer", "gradientMap", "selectiveColor"];

#[test]
fn preview_composite_matches_the_destructive_command() {
    let tol = 1.0 / 255.0 + 1e-4;
    let mut worst = (0.0f32, String::new());
    let mut fails = Vec::new();
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for selection in [false, true] {
                let (doc, target) = document(mode, depth, selection);
                for kind in KINDS {
                    for params in [sample(kind), json!({})] {
                        let what = format!("{kind} {mode:?} {depth:?} selection {selection} {params}");
                        let want = destructive(&doc, target, kind, &params);
                        let got = preview_document(&doc, target, kind, &params);
                        let gray_colour = mode == ColorMode::Grayscale && GRAY_COLOUR.contains(&kind);
                        let (Some(want), Ok(got)) = (want, got) else {
                            assert!(gray_colour, "{what}: command and preview disagree on validity");
                            continue;
                        };
                        assert!(!gray_colour, "{what}: should use the proxy preview");
                        let d = max_diff(&got, &want);
                        if d > worst.0 {
                            worst = (d, what.clone());
                        }
                        if d > tol {
                            fails.push(format!("{what}: preview differs from the command by {:.2}/255", d * 255.0));
                        }
                    }
                }
            }
        }
    }
    eprintln!("worst preview difference {:.3}/255 ({})", worst.0 * 255.0, worst.1);
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// A clipped target is adjusted through an isolating group in the preview only: its composite
/// matches the destructive command for every kind, depth, mode and selection.
#[test]
fn a_clipped_target_previews_like_the_destructive_command() {
    let tol = 1.0 / 255.0 + 1e-4;
    let mut worst = (0.0f32, String::new());
    let mut fails = Vec::new();
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
            for selection in [false, true] {
                let (doc, target) = clipped_document(mode, depth, selection);
                assert!(unsupported(&doc, target).is_none(), "a clipped raster target is supported");
                for kind in KINDS {
                    for params in [sample(kind), json!({})] {
                        let what = format!("{kind} {mode:?} {depth:?} selection {selection} {params}");
                        let want = destructive(&doc, target, kind, &params);
                        let got = preview_document(&doc, target, kind, &params);
                        let gray_colour = mode == ColorMode::Grayscale && GRAY_COLOUR.contains(&kind);
                        let (Some(want), Ok(got)) = (want, got) else {
                            assert!(gray_colour, "{what}: command and preview disagree on validity");
                            continue;
                        };
                        assert!(!gray_colour, "{what}: should use the proxy preview");
                        let d = max_diff(&got, &want);
                        if d > worst.0 {
                            worst = (d, what.clone());
                        }
                        if d > tol {
                            fails.push(format!("{what}: preview differs from the command by {:.2}/255", d * 255.0));
                        }
                    }
                }
            }
        }
    }
    eprintln!("worst clipped-target preview difference {:.3}/255 ({})", worst.0 * 255.0, worst.1);
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// The isolating preview keeps the target's place and compositing properties, puts its raw raster
/// and the adjustment inside its group, and leaves the committed document untouched.
#[test]
fn a_clipped_target_preview_keeps_its_place_and_properties() {
    let (mut doc, target) = clipped_document(ColorMode::Rgb, SampleType::U8, true);
    // Nondefaults the transformed group must keep, not just Photoshop's defaults.
    let l = doc.layer_mut(target).unwrap();
    l.fill_opacity = 0.4;
    l.excluded_channels = 0b101;
    l.advanced.blend_clipped = false;
    let before = doc.layer(target).unwrap().clone();
    let path = doc.path_of(target).unwrap();
    let layer_count = doc.layer_count();
    let base = base_document(&doc, target).unwrap();
    let group = base.layer(target).unwrap();
    assert!(group.is_group(), "the target becomes an isolating group");
    assert_eq!(base.path_of(target), Some(path.clone()), "same place in the stack");
    assert_eq!(group.clipped, before.clipped);
    assert_eq!((group.blend, group.opacity, group.fill_opacity), (before.blend, before.opacity, before.fill_opacity));
    assert_eq!(group.mask, before.mask);
    assert_eq!(group.effects, before.effects);
    assert_eq!(group.excluded_channels, before.excluded_channels);
    assert_eq!(group.advanced, before.advanced);
    let children = group.children().unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].id, CARRIER_LAYER);
    assert_eq!(children[0].surface(), before.surface(), "the raw raster is unchanged");
    assert_eq!(children[1].id, PREVIEW_LAYER);
    assert!(children[1].clipped, "the adjustment clips to the raw raster");
    let mut preview_path = path;
    preview_path.push(1);
    assert_eq!(base.path_of(PREVIEW_LAYER), Some(preview_path), "inside the target's group");
    // The committed document is untouched.
    assert_eq!(doc.layer_count(), layer_count);
    assert!(doc.layer(PREVIEW_LAYER).is_none());
    assert!(doc.layer(CARRIER_LAYER).is_none());
    assert_eq!(doc.layer(target).unwrap().surface(), before.surface());
    // The reduced-resolution preview flips every id, including the carrier and the nested
    // adjustment, so its textures stay separate and stable.
    let proxy = proxy_base(&base, 2);
    assert_ne!(proxy.id, base.id);
    assert!(proxy.walk().iter().all(|(_, _, l)| base.layer(l.id).is_none() && l.id != PREVIEW_LAYER));
    assert!(proxy.layer(LayerId(PREVIEW_LAYER.0 ^ PROXY_LAYER_BIT)).is_some());
    assert!(proxy.layer(LayerId(CARRIER_LAYER.0 ^ PROXY_LAYER_BIT)).is_some());
}

/// The canvas damage region still bounds every change when the target is clipped: outside it the
/// preview composites like the committed document, within a tight premultiplied tolerance.
#[test]
fn a_clipped_target_preview_changes_nothing_outside_its_region() {
    // The extra group's straight-alpha normalization introduces floating-point rounding.
    // Allow 1e-6 outside the damage region, well below the 1/255 command-parity tolerance;
    // the committed document itself must remain identical.
    const OUTSIDE: f32 = 1e-6;
    let pm = |p: &[f32; 4]| [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]];
    for selection in [false, true] {
        let (doc, target) = clipped_document(ColorMode::Rgb, SampleType::U8, selection);
        let committed = doc.clone();
        let r = region(&doc, target);
        assert!(!r.is_empty(), "the target can change something");
        let before = photocraft_compose::render(&doc, doc.bounds());
        for kind in KINDS {
            let p = preview_document(&doc, target, kind, &sample(kind)).unwrap();
            let after = photocraft_compose::render(&p, doc.bounds());
            for (i, (a, b)) in before.px.iter().zip(&after.px).enumerate() {
                let (x, y) = ((i % 40) as i32, (i / 40) as i32);
                if !r.contains(x, y) {
                    let (pa, pb) = (pm(a), pm(b));
                    assert!((0..4).all(|c| (pa[c] - pb[c]).abs() <= OUTSIDE), "{kind} selection {selection}: ({x}, {y}) outside {r:?}: {pa:?} vs {pb:?}");
                }
            }
        }
        assert_eq!(doc, committed, "the committed document is unchanged");
    }
}

#[test]
fn preview_leaves_the_document_alone_and_marks_its_layer() {
    let (doc, target) = document(ColorMode::Rgb, SampleType::U8, true);
    let p = preview_document(&doc, target, "curves", &sample("curves")).unwrap();
    assert_eq!(p.layer_count(), doc.layer_count() + 1);
    let l = p.layer(PREVIEW_LAYER).unwrap();
    assert!(l.clipped && l.mask.is_some());
    // Directly above the target, below the layer already clipped to it.
    let path = p.path_of(PREVIEW_LAYER).unwrap();
    assert_eq!(p.layer_at(&[path[0] - 1]).unwrap().id, target);
    assert!(doc.layer(PREVIEW_LAYER).is_none());
    // Region: the target's content within the selection.
    assert_eq!(region(&doc, target), Rect::from_xywh(8, 4, 24, 20));
}

#[test]
fn ineligible_targets_use_the_proxy_preview() {
    let (mut doc, target) = document(ColorMode::Rgb, SampleType::U8, false);
    let clipped = doc.layers[2].id;
    assert_eq!(unsupported(&doc, clipped), None, "a clipped raster target previews by isolating it");
    assert!(base_document(&doc, clipped).unwrap().layer(clipped).unwrap().is_group());
    // Blend If still needs the CPU proxy: its "This Layer" ranges test the adjusted colour.
    let (mut blend_doc, blend_target) = clipped_document(ColorMode::Rgb, SampleType::U8, false);
    let mut blend_if = BlendIf::default();
    blend_if.set(0, [BlendRange { black: [30, 30], white: [220, 220] }, BlendRange::FULL]);
    blend_doc.layer_mut(blend_target).unwrap().blend_if = blend_if;
    assert_eq!(unsupported(&blend_doc, blend_target), Some(tl!("Blend If")));
    assert_eq!(unsupported(&doc, LayerId(u64::MAX - 1)), Some("no target layer"));
    doc.layers.push(Layer::new("adj", LayerContent::Adjustment(photocraft_doc::Adjustment::Invert)));
    let adj = doc.layers[4].id;
    assert_eq!(unsupported(&doc, adj), Some("not a pixel layer"));
    assert!(unsupported(&doc, target).is_none());
    // Bad settings are an error, not a panic.
    assert!(preview_document(&doc, target, "levels", &json!({"gamma": "x"})).is_err());
    assert!(preview_document(&doc, target, "nope", &json!({})).is_err());
    assert!(preview_document(&doc, target, "curves", &json!([1, 2])).is_err());
    // CMYK and Lab adjust through RGB per pixel: proxy preview.
    for mode in [ColorMode::Cmyk, ColorMode::Lab] {
        let (d, t) = document(mode, SampleType::U8, false);
        assert!(unsupported(&d, t).is_some(), "{mode:?}");
    }
    let (gray, t) = document(ColorMode::Grayscale, SampleType::U16, false);
    assert!(preview_document(&gray, t, "hueSaturation", &json!({"colorize": true, "hue": 30, "saturation": 40})).is_err());
    assert!(preview_document(&gray, t, "hueSaturation", &json!({"lightness": 20})).is_ok());
}

/// A clipped target with enabled effects on itself or an enclosing group keeps the CPU proxy:
/// the preview's temporary group would rebuild the styled group's effect shape maps on the UI
/// thread for every settings change. Disabled effects and styled siblings elsewhere don't count.
#[test]
fn a_styled_clipped_target_keeps_the_proxy_preview() {
    let overlay = || Effect::ColorOverlay { common: FxCommon::new(BlendMode::Normal, 0.5), color: Color::rgba(0.2, 0.8, 0.4, 1.0) };
    let (mut doc, target) = clipped_document(ColorMode::Rgb, SampleType::U8, false);
    assert_eq!(unsupported(&doc, target), None, "the fixture target is unstyled");
    // An enabled effect on the target itself.
    doc.layer_mut(target).unwrap().effects.items.push(overlay());
    let why = unsupported(&doc, target).expect("a styled clipped target keeps the proxy");
    assert!(why.contains("layer effects"), "{why}");
    assert!(base_document(&doc, target).is_err(), "no preview group for a styled clip");
    doc.layer_mut(target).unwrap().clipped = false;
    assert_eq!(unsupported(&doc, target), None, "unclipped styled targets keep their existing path");
    doc.layer_mut(target).unwrap().clipped = true;
    doc.layer_mut(target).unwrap().effects.enabled = false;
    assert_eq!(unsupported(&doc, target), None, "the disabled master switch doesn't count");
    doc.layer_mut(target).unwrap().effects.enabled = true;
    // Disabled effects cost nothing: still eligible.
    if let Effect::ColorOverlay { common, .. } = &mut doc.layer_mut(target).unwrap().effects.items[0] {
        common.enabled = false;
    }
    assert_eq!(unsupported(&doc, target), None, "disabled effects don't count");
    doc.layer_mut(target).unwrap().effects.items.clear();
    // An enabled effect on an enclosing group.
    let inner = doc.layers[1].children().unwrap()[0].id;
    doc.layer_mut(inner).unwrap().effects.items.push(overlay());
    assert!(unsupported(&doc, target).is_some(), "a styled enclosing group keeps the proxy");
    if let Effect::ColorOverlay { common, .. } = &mut doc.layer_mut(inner).unwrap().effects.items[0] {
        common.enabled = false;
    }
    assert_eq!(unsupported(&doc, target), None, "a disabled group effect doesn't count");
    doc.layer_mut(inner).unwrap().effects.items.clear();
    // A styled sibling inside the same clipping group is unrelated.
    let above = doc.layer(inner).unwrap().children().unwrap()[3].id;
    doc.layer_mut(above).unwrap().effects.items.push(overlay());
    assert_eq!(unsupported(&doc, target), None, "an unrelated styled sibling doesn't reject the target");
    assert!(base_document(&doc, target).is_ok());
}

/// The canvas recomposites only `region` between previews: nothing outside it may change.
#[test]
fn preview_changes_nothing_outside_its_region() {
    for selection in [false, true] {
        let (doc, target) = document(ColorMode::Rgb, SampleType::U8, selection);
        let r = region(&doc, target);
        let before = photocraft_compose::render(&doc, doc.bounds());
        for kind in KINDS {
            let p = preview_document(&doc, target, kind, &sample(kind)).unwrap();
            let after = photocraft_compose::render(&p, doc.bounds());
            for (i, (a, b)) in before.px.iter().zip(&after.px).enumerate() {
                let (x, y) = ((i % 40) as i32, (i / 40) as i32);
                if !r.contains(x, y) {
                    assert_eq!(a, b, "{kind} selection {selection}: ({x}, {y}) outside {r:?}");
                }
            }
        }
    }
}

fn app_with(doc: Document, target: LayerId) -> PhotocraftApp {
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    s.select_layer(target).unwrap();
    PhotocraftApp::new(s, crate::Services::default())
}

/// The dialog previews through the layer (no CPU proxy run), each change only swaps the
/// preview document; Cancel leaves the document untouched, OK is one history step.
#[test]
fn dialog_previews_through_the_layer() {
    use egui_kittest::Harness;
    for confirm in [false, true] {
        let (doc, target) = document(ColorMode::Rgb, SampleType::U16, true);
        let mut harness =
            Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app_with(doc, target));
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        let id = crate::adjust_dialog::open(harness.state_mut(), "image.adjustments.levels").unwrap();
        harness.run_steps(2);
        let committed = harness.state().session.active().unwrap().doc.clone();
        let steps = harness.state().session.active().unwrap().history.past_len();
        let app = harness.state_mut();
        assert!(on_layer(app, 0));
        let (first, k1) = display_doc(app, 0).unwrap();
        assert!(first.layer(PREVIEW_LAYER).is_some());
        // Unchanged settings: the cached preview, same key.
        let (again, k1b) = display_doc(app, 0).unwrap();
        assert!(Arc::ptr_eq(&first, &again) && k1 == k1b);
        // A change: new key, and the canvas only recomposites the target's area.
        let d = app.ui.dialog_mut(id).unwrap();
        d.fields.insert("gamma".into(), json!(1.6));
        let (second, k2) = display_doc(app, 0).unwrap();
        assert_ne!(k1, k2);
        let rev = app.session.active().unwrap().revision;
        let doc_id = committed.id;
        assert_eq!(switch_region(app, doc_id, rev, k1, k2), Some(Rect::from_xywh(8, 4, 24, 20)));
        assert_eq!(switch_region(app, doc_id, rev, 0, k2), Some(Rect::from_xywh(8, 4, 24, 20)));
        assert_eq!(switch_region(app, doc_id, rev, 1 << 41, k2), None, "another preview kind");
        assert_eq!(switch_region(app, doc_id, rev + 1, k1, k2), None, "another revision");
        // Only the visible part composites now; the rest when the view gets there.
        let region = Rect::from_xywh(8, 4, 24, 20);
        let (corner, all) = (Rect::from_xywh(0, 0, 16, 16), Rect::from_xywh(0, 0, 40, 32));
        assert_eq!(switch_damage(app, k2, region, corner), Rect::new(8, 4, 16, 16));
        assert_eq!(uncovered(app, doc_id, k2, corner), None);
        assert_eq!(uncovered(app, doc_id, k1, all), None, "another preview");
        assert_eq!(uncovered(app, doc_id, k2, all), Some(region));
        assert_eq!(uncovered(app, doc_id, k2, all), None);
        assert_eq!(switch_damage(app, 0, region, corner), region, "back to the document: everything");
        assert_eq!(uncovered(app, doc_id, k2, all), None);
        // The preview equals the command with the dialog's settings.
        let params = crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields);
        let want = destructive(&committed, target, "levels", &params).unwrap();
        assert!(max_diff(&second, &want) <= 1.0 / 255.0);
        // Preview off: the committed document.
        app.ui.dialog_mut(id).unwrap().fields.insert("__preview".into(), json!(false));
        assert!(display_doc(app, 0).is_none());
        assert!(on_layer(app, 0), "still the layer path, just hidden");
        harness.run_steps(2);
        assert!(Arc::ptr_eq(&harness.state().session.active().unwrap().doc, &committed), "previewing leaves the document alone");
        if confirm {
            crate::dialogs::confirm(harness.state_mut(), id).unwrap();
            let st = harness.state().session.active().unwrap();
            assert_eq!(st.history.past_len(), steps + 1, "one history step");
            assert!(st.doc.layer(PREVIEW_LAYER).is_none());
            assert!(max_diff(&st.doc, &want) <= 1e-6, "OK runs the real command");
        } else {
            harness.state_mut().ui.close_dialog(id);
            harness.run_steps(2);
            let st = harness.state().session.active().unwrap();
            assert!(Arc::ptr_eq(&st.doc, &committed), "Cancel leaves the document untouched");
            assert_eq!(st.history.past_len(), steps);
        }
        let app = harness.state_mut();
        assert!(display_doc(app, 0).is_none() && !on_layer(app, 0));
    }
}

/// A clipped target still previews through the layer: the session keys, damage region and
/// settings swap work on its isolating group, and closing the dialog drops the preview.
#[test]
fn a_clipped_target_dialog_previews_through_the_layer() {
    let (doc, target) = clipped_document(ColorMode::Rgb, SampleType::U16, true);
    let mut app = app_with(doc.clone(), target);
    let id = crate::adjust_dialog::open(&mut app, "image.adjustments.levels").unwrap();
    assert!(on_layer(&mut app, 0));
    let (first, k1) = display_doc(&mut app, 0).unwrap();
    assert!(first.layer(target).unwrap().is_group());
    assert!(first.layer(PREVIEW_LAYER).is_some());
    let (again, k1b) = display_doc(&mut app, 0).unwrap();
    assert!(Arc::ptr_eq(&first, &again) && k1 == k1b, "unchanged settings reuse the cached preview");
    app.ui.dialog_mut(id).unwrap().fields.insert("gamma".into(), json!(1.6));
    let (second, k2) = display_doc(&mut app, 0).unwrap();
    assert_ne!(k1, k2);
    let st = app.session.active().unwrap();
    assert_eq!(switch_region(&app, st.doc.id, st.revision, k1, k2), Some(region(&doc, target)));
    let params = crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields);
    let want = destructive(&doc, target, "levels", &params).unwrap();
    assert!(max_diff(&second, &want) <= 1.0 / 255.0);
    app.ui.dialog_mut(id).unwrap().fields.insert("__preview".into(), json!(false));
    assert!(display_doc(&mut app, 0).is_none());
    assert!(on_layer(&mut app, 0), "still the layer path, just hidden");
    app.ui.close_dialog(id);
    assert!(display_doc(&mut app, 0).is_none() && !on_layer(&mut app, 0));
}

/// Targets the layer path can't reproduce keep the CPU proxy preview.
#[test]
fn ineligible_dialogs_keep_the_proxy_preview() {
    // An adjustment-layer target.
    let (mut doc, _) = document(ColorMode::Rgb, SampleType::U8, false);
    doc.layers.push(Layer::new("adj", LayerContent::Adjustment(photocraft_doc::Adjustment::Invert)));
    let adj = doc.layers[4].id;
    let mut app = app_with(doc, adj);
    crate::adjust_dialog::open(&mut app, "image.adjustments.curves").unwrap();
    assert!(!on_layer(&mut app, 0) && display_doc(&mut app, 0).is_none());
    // An alpha-channel target.
    let (doc, target) = document(ColorMode::Rgb, SampleType::U8, false);
    let mut app = app_with(doc, target);
    app.session.execute("select.all", json!({})).unwrap();
    app.session.execute("select.saveSelection", json!({})).unwrap();
    if let Some(st) = app.session.active_mut() {
        st.channel_view.target = photocraft_engine::channel_cmds::ChannelTarget::Alpha(0);
    }
    crate::adjust_dialog::open(&mut app, "image.adjustments.levels").unwrap();
    assert!(!on_layer(&mut app, 0));
}

/// The proxy preview of an ineligible target records why it doesn't preview through the layer
/// instead of dropping the reason (#7487).
#[test]
fn an_ineligible_adjustment_preview_records_why_it_uses_the_proxy() {
    let (mut doc, _) = document(ColorMode::Rgb, SampleType::U8, false);
    doc.layers.push(Layer::new("adj", LayerContent::Adjustment(photocraft_doc::Adjustment::Invert)));
    let adj = doc.layers[4].id;
    let mut app = app_with(doc, adj);
    app.render_path.assume_gpu = true;
    let dialog = crate::adjust_dialog::open(&mut app, "image.adjustments.curves").unwrap();
    assert!(!on_layer(&mut app, 0));
    let doc = app.session.active().unwrap().doc.id.0;
    assert_eq!(app.render_path.current(doc), Some(&crate::render_path::CpuReason::AdjustPreview("not a pixel layer")));
    let path = crate::render_path::inspect(&app);
    assert_eq!(path["current"]["kind"], "adjustPreview");
    assert!(path["current"]["detail"].as_str().is_some_and(|d| d.contains("not a pixel layer")), "{path}");
    // Asked again with nothing changed: no new entry.
    assert!(!on_layer(&mut app, 0));
    assert_eq!(app.render_path.history().count(), 1);
    // A GPU refresh of the canvas while the dialog is open doesn't end the preview's reason.
    crate::render_path::note(&mut app, doc, crate::render_path::Path::Gpu { full: true });
    assert_eq!(app.render_path.current(doc), Some(&crate::render_path::CpuReason::AdjustPreview("not a pixel layer")));
    assert_eq!(app.render_path.history().count(), 1);
    // Closing the dialog ends the reason, though the canvas is still current (no refresh).
    app.ui.close_dialog(dialog);
    assert!(!on_layer(&mut app, 0));
    assert_eq!(app.render_path.current(doc), None);
    assert_eq!(app.render_path.history().count(), 2);
    let path = crate::render_path::inspect(&app);
    assert_eq!(path["history"][1]["kind"], "adjustPreview", "the preview's end isn't the canvas back on the GPU: {path}");
    assert!(path["history"][1]["detail"].is_null(), "{path}");
}

/// A document the canvas already draws on the CPU keeps that reason through an adjustment
/// dialog's proxy preview, and closing the dialog doesn't report "back on the GPU".
#[test]
fn an_adjustment_preview_keeps_the_canvas_reason() {
    use crate::render_path::{CpuReason, Path};
    let (mut doc, _) = document(ColorMode::Rgb, SampleType::U8, false);
    doc.layers.push(Layer::new("adj", LayerContent::Adjustment(photocraft_doc::Adjustment::Invert)));
    let adj = doc.layers[4].id;
    let canvas_back_on_gpu = |app: &crate::PhotocraftApp| app.render_path.history().any(|c| !c.preview && c.reason.is_none());
    let mut app = app_with(doc.clone(), adj);
    app.render_path.assume_gpu = true;
    let doc_id = app.session.active().unwrap().doc.id.0;
    let canvas = CpuReason::Unsupported("Advanced Blending on Layer 1".into());
    crate::render_path::note(&mut app, doc_id, Path::Cpu(canvas.clone()));
    let dialog = crate::adjust_dialog::open(&mut app, "image.adjustments.curves").unwrap();
    assert!(!on_layer(&mut app, 0));
    assert_eq!(app.render_path.current(doc_id), Some(&canvas));
    app.ui.close_dialog(dialog);
    assert!(!on_layer(&mut app, 0));
    assert_eq!(app.render_path.current(doc_id), Some(&canvas));
    // The canvas entry, then the preview's start and end: never the canvas back on the GPU.
    assert_eq!(app.render_path.history().filter(|c| !c.preview).count(), 1);
    assert!(!canvas_back_on_gpu(&app));

    // A window without the GPU (`--safe-gpu`): the preview adds nothing, before or after closing.
    let mut app = app_with(doc, adj);
    app.render_path.assume_gpu = true;
    let doc_id = app.session.active().unwrap().doc.id.0;
    let cpu = CpuReason::CpuMode("--safe-gpu".into());
    crate::render_path::note_app(&mut app, Some(cpu.clone()));
    let dialog = crate::adjust_dialog::open(&mut app, "image.adjustments.curves").unwrap();
    assert!(!on_layer(&mut app, 0));
    assert_eq!(app.render_path.current(doc_id), Some(&cpu));
    app.ui.close_dialog(dialog);
    assert!(!on_layer(&mut app, 0));
    assert_eq!(app.render_path.current(doc_id), Some(&cpu));
    assert_eq!(app.render_path.history().count(), 1, "the app-wide entry only");
    assert!(!canvas_back_on_gpu(&app));
}

#[test]
fn proxy_factor_follows_the_zoom() {
    assert_eq!(proxy_factor(1.0), 1);
    assert_eq!(proxy_factor(0.6), 1);
    assert_eq!(proxy_factor(0.5), 2);
    assert_eq!(proxy_factor(0.2), 4);
    assert_eq!(proxy_factor(0.1), 8);
    assert_eq!(proxy_factor(0.0001), 64);
    assert_eq!(proxy_factor(0.0), 1);
    assert_eq!(proxy_factor(f32::NAN), 1);
}

/// The zoomed-out proxy keeps the preview layer (with the dialog's settings) under its own ids.
#[test]
fn proxy_preview_has_its_own_ids() {
    let (doc, target) = document(ColorMode::Rgb, SampleType::U8, true);
    let base = base_document(&doc, target).unwrap();
    let proxy = proxy_base(&base, 2);
    assert_eq!((proxy.size.width, proxy.size.height), (20, 16));
    assert_ne!(proxy.id, doc.id);
    assert!(proxy.walk().iter().all(|(_, _, l)| doc.layer(l.id).is_none() && l.id != PREVIEW_LAYER));
    let p = proxy_with_settings(&proxy, "curves", &sample("curves")).unwrap();
    let full = with_settings(&base, "curves", &sample("curves")).unwrap();
    let (a, b) = (photocraft_compose::render(&p, p.bounds()), photocraft_compose::render(&full, full.bounds()));
    // Nearest-neighbour proxy: pixel (x, y) shows the full composite at (2x, 2y).
    for y in 0..16 {
        for x in 0..20 {
            let (pa, pb) = (a.px[y * 20 + x], b.px[y * 2 * 40 + x * 2]);
            assert!((0..4).all(|c| (pa[c] - pb[c]).abs() < 2.0 / 255.0), "({x}, {y}) {pa:?} vs {pb:?}");
        }
    }
    assert!(proxy_with_settings(&proxy, "levels", &json!({"gamma": "x"})).is_err());
}

/// A Properties-panel drag damages the layer's area and only the part of it the view shows,
/// catching up as the view moves; a new document revision ends the session (#6839).
#[test]
fn a_live_adjust_drag_damages_only_the_visible_part_of_its_layer() {
    let (doc, target) = document(ColorMode::Rgb, SampleType::U8, false);
    let (id, bounds) = (doc.id, doc.bounds());
    let mut app = app_with(doc.clone(), target);

    let k1 = live_key(&mut app, &doc, 7, target, &json!({"gamma": 1.1}));
    let k2 = live_key(&mut app, &doc, 7, target, &json!({"gamma": 1.4}));
    assert_ne!(k1, k2, "a different setting is a different key");
    assert_eq!(k1 & LIVE_TAG, LIVE_TAG, "keys carry the live tag");
    assert_eq!(k1 >> LIVE_SESSION_SHIFT, k2 >> LIVE_SESSION_SHIFT, "the same drag is one session");
    assert_ne!(k1 >> LIVE_SESSION_SHIFT, TAG >> LIVE_SESSION_SHIFT, "never the dialog's tag");

    // Between two of the drag's previews, and between one and the committed document.
    let region = live_switch_region(&app, id, 7, k1, k2).expect("same session");
    assert_eq!(region, bounds, "the region is the whole document");
    assert!(live_switch_region(&app, id, 7, 0, k2).is_some(), "from the committed document");
    assert!(live_switch_region(&app, id, 7, k1, 0).is_some(), "back to the committed document");
    assert!(live_switch_region(&app, id, 7, 0, 0).is_none(), "the document to itself is no switch");
    assert!(live_switch_region(&app, id, 8, k1, k2).is_none(), "another revision is another session");

    // Only the visible part is composited now; moving the view asks for the rest.
    let mid = region.x0 + region.width() as i32 / 2;
    let left = Rect::new(region.x0, region.y0, mid, region.y1);
    assert_eq!(live_switch_damage(&mut app, k2, region, left), region.intersect(&left));
    assert!(live_uncovered(&mut app, id, k2, left).is_none(), "what was just composited is covered");
    let right = Rect::new(mid, region.y0, region.x1, region.y1);
    assert_eq!(live_uncovered(&mut app, id, k2, right), Some(region.intersect(&right)), "the view moved");
    assert!(live_uncovered(&mut app, id, k2, right).is_none(), "and is covered once composited");
    assert!(live_uncovered(&mut app, id, k1, right).is_none(), "another preview's coverage is not this one's");

    // Switching back to the document composites the whole region, not just the view.
    assert_eq!(live_switch_damage(&mut app, 0, region, left), region);
}

/// A reveal-all mask stores its coverage in the surface's default pixel and reports an empty
/// `content_bounds()`. Narrowing the region by it emptied the damage and the canvas drew nothing
/// for a whole drag — every adjustment layer `layer.newAdjustmentLayer` creates has one (#6839).
#[test]
fn a_reveal_all_mask_does_not_empty_the_damage_region() {
    let (mut doc, target) = document(ColorMode::Rgb, SampleType::U8, false);
    let bounds = doc.bounds();
    let mask = photocraft_doc::LayerMask::reveal_all();
    assert!(mask.surface.content_bounds().is_empty(), "a reveal-all mask has no written content");
    doc.layer_mut(target).expect("target").mask = Some(mask);
    let id = doc.id;
    let mut app = app_with(doc.clone(), target);

    let k1 = live_key(&mut app, &doc, 3, target, &json!({"gamma": 1.1}));
    let k2 = live_key(&mut app, &doc, 3, target, &json!({"gamma": 1.4}));
    let region = live_switch_region(&app, id, 3, k1, k2).expect("same session");
    assert_eq!(region, bounds, "the whole document, not the mask's empty content");
    assert!(!live_switch_damage(&mut app, k2, region, bounds).is_empty(), "something is composited");
}

#[test]
fn captured_subset_preview_stays_on_the_command_path_when_the_panel_changes() {
    for kind in ["levels", "curves"] {
        let (doc, target) = document(ColorMode::Rgb, SampleType::U16, false);
        let mut app = app_with(doc, target);
        app.session.execute("channel.target", json!({"colors":[0,2]})).unwrap();
        let command = format!("image.adjustments.{kind}");
        let id = crate::adjust_dialog::open(&mut app, &command).unwrap();
        if let Some(values) = sample(kind).as_object() {
            app.ui.dialog_mut(id).unwrap().fields.extend(values.clone());
        }
        app.session.execute("channel.target", json!({"channel":"composite"})).unwrap();
        assert!(!on_layer(&mut app, 0), "captured colour set cannot use an all-channel adjustment layer");
        let params = crate::filter_dialog::params_of(&app.ui.dialog_mut(id).unwrap().fields);
        let doc = app.session.active().unwrap().doc.clone();
        let preview = crate::filter_dialog::preview_document(&doc, Some(target), &command, &params, 1).unwrap();
        let expected = destructive(&doc, target, kind, &params).unwrap();
        assert!(max_diff(&preview, &expected) <= 1e-6);
        app.session.execute("channel.target", json!({"colors":[0,2]})).unwrap();
        app.ui.dialog_mut(id).unwrap().fields.insert("target".into(), json!("pixels"));
        assert!(on_layer(&mut app, 0), "explicit composite target overrides the panel too");
    }
}
