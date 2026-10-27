//! Select › Color Range… as Photoshop lays it out: the Select menu (Sampled Colors, the six hue
//! families, Highlights / Midtones / Shadows, Out Of Gamut), Fuzziness, Localized Color Clusters
//! with Range, the tonal range of Highlights / Midtones / Shadows, the three eyedroppers (sample,
//! add, subtract; they pick on the preview, Shift adds and Alt subtracts), a Selection / Image
//! preview, canvas Selection Preview modes and Invert. Controls a mode doesn't use are disabled, like Photoshop's.
//!
//! Everything lives in the dialog fields (`ui.dialog.set` drives it). The preview and OK run the
//! same engine command, `select.colorRange`, so the GUI, MCP and scripts select identically; the
//! preview compares a small proxy against samples from the original document; OK selects at
//! full resolution (one history step). Reduced previews approximate fine detail and effects.
//! Cancel never touches the document, so the previous selection stays as it was.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::{DocId, Document};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::DialogKind;
use crate::theme::Tokens;
use crate::widgets;

/// The engine command the dialog drives.
pub const COMMAND: &str = "select.colorRange";

/// Select menu entries in Photoshop's order (engine value, label).
pub const SELECTS: &[(&str, &str)] = &[
    ("sampledColors", "Sampled Colors"),
    ("reds", "Reds"),
    ("yellows", "Yellows"),
    ("greens", "Greens"),
    ("cyans", "Cyans"),
    ("blues", "Blues"),
    ("magentas", "Magentas"),
    ("highlights", "Highlights"),
    ("midtones", "Midtones"),
    ("shadows", "Shadows"),
    ("skinTones", "Skin Tones"),
    ("outOfGamut", "Out Of Gamut"),
];

/// Longest side of the preview thumbnail, in points.
const PREVIEW: u32 = 200;
/// Canvas overlays are view state; keep their allocation bounded on very large documents.
const CANVAS_PREVIEW: u32 = 1024;
const PREVIEWS: &[(&str, &str)] =
    &[("none", "None"), ("grayscale", "Grayscale"), ("blackMatte", "Black Matte"), ("whiteMatte", "White Matte"), ("quickMask", "Quick Mask")];

/// Which controls the current Select mode uses (the rest are disabled or hidden).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Controls {
    /// Fuzziness (Sampled Colors: colour distance; tones: falloff in % of the tonal scale).
    pub fuzziness: bool,
    /// The eyedroppers and Localized Color Clusters (Sampled Colors only).
    pub sampling: bool,
    /// The Range slider (Localized Color Clusters on).
    pub range: bool,
    /// The tonal range sliders (Highlights / Midtones / Shadows).
    pub tonal: bool,
}

fn s<'a>(f: &'a Map<String, Value>, k: &str, d: &'a str) -> &'a str {
    f.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn num(f: &Map<String, Value>, k: &str, d: f32) -> f32 {
    f.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn flag(f: &Map<String, Value>, k: &str) -> bool {
    f.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn points(f: &Map<String, Value>, k: &str) -> Vec<[f64; 2]> {
    let pt = |v: &Value| match v.as_array()?.as_slice() {
        [x, y] => Some([x.as_f64()?, y.as_f64()?]),
        _ => None,
    };
    f.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(pt).collect()).unwrap_or_default()
}

pub fn controls(f: &Map<String, Value>) -> Controls {
    let select = s(f, "select", "sampledColors");
    let sampled = select == "sampledColors";
    let tonal = matches!(select, "highlights" | "midtones" | "shadows");
    Controls { fuzziness: sampled || tonal || select == "skinTones", sampling: sampled, range: sampled && flag(f, "localized"), tonal }
}

/// Open the dialog with Photoshop's defaults (Sampled Colors, Fuzziness 40, the foreground colour
/// as the sample until the eyedropper picks one).
pub fn open(app: &mut PhotocraftApp) -> u64 {
    let label = photocraft_engine::commands::find(COMMAND).map_or(tl!("Color Range…"), |c| c.label);
    let mut f = Map::new();
    f.insert("__colorRange".into(), json!(true));
    f.insert("__label".into(), json!(label));
    f.insert("select".into(), json!("sampledColors"));
    f.insert("fuzziness".into(), json!(40.0));
    f.insert("localized".into(), json!(false));
    f.insert("range".into(), json!(100.0));
    f.insert("toneFuzziness".into(), json!(20.0));
    f.insert("shadowsLevel".into(), json!(65.0));
    f.insert("highlightsLevel".into(), json!(190.0));
    f.insert("midtonesLow".into(), json!(105.0));
    f.insert("midtonesHigh".into(), json!(150.0));
    f.insert("invert".into(), json!(false));
    f.insert("points".into(), json!([]));
    f.insert("subtractPoints".into(), json!([]));
    f.insert("__order".into(), json!([]));
    f.insert("__tool".into(), json!("sample"));
    f.insert("__view".into(), json!("selection"));
    f.insert("__selectionPreview".into(), json!("none"));
    app.color_range = None;
    app.ui.open_dialog(DialogKind::Command, f)
}

pub fn owns(f: &Map<String, Value>) -> bool {
    f.contains_key("__colorRange")
}

/// The `select.colorRange` params for the dialog's state: only what the Select mode uses.
pub fn params(f: &Map<String, Value>) -> Value {
    let select = s(f, "select", "sampledColors");
    let c = controls(f);
    let mut p = Map::new();
    p.insert("select".into(), json!(select));
    p.insert("invert".into(), json!(flag(f, "invert")));
    if c.sampling {
        p.insert("fuzziness".into(), json!(num(f, "fuzziness", 40.0)));
        let (add, sub) = (points(f, "points"), points(f, "subtractPoints"));
        let picked = !add.is_empty() || !sub.is_empty();
        if !add.is_empty() {
            p.insert("points".into(), json!(add));
        }
        if !sub.is_empty() {
            p.insert("subtractPoints".into(), json!(sub));
            if let Some(o) = order(f, add.len(), sub.len()) {
                p.insert("order".into(), o);
            }
        }
        // Localized clusters need a picked position; until then it is the plain colour range.
        if c.range && picked {
            p.insert("localized".into(), json!(true));
            p.insert("range".into(), json!(num(f, "range", 100.0)));
        }
    } else if select == "skinTones" {
        p.insert("fuzziness".into(), json!(num(f, "fuzziness", 40.0)));
    } else if c.tonal {
        p.insert("fuzziness".into(), json!(num(f, "toneFuzziness", 20.0)));
        let range = match select {
            "shadows" => json!(num(f, "shadowsLevel", 65.0)),
            "highlights" => json!(num(f, "highlightsLevel", 190.0)),
            _ => {
                let (lo, hi) = (num(f, "midtonesLow", 105.0), num(f, "midtonesHigh", 150.0));
                json!([lo.min(hi), lo.max(hi)])
            }
        };
        p.insert("tonalRange".into(), range);
    }
    Value::Object(p)
}

/// OK: run the command on the document (one history step).
pub fn confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    app.color_range = None;
    app.run(COMMAND, params(f))
}

/// Cached preview: the document proxy (per revision) and the textures drawn from it.
pub struct Preview {
    doc: DocId,
    revision: u64,
    /// Proxy scale: proxy pixel = `k` document pixels.
    k: u32,
    proxy: Arc<Document>,
    image: Option<egui::TextureHandle>,
    /// Hash of the params the mask texture shows.
    mask_key: u64,
    mask: Option<egui::TextureHandle>,
    /// Why the preview couldn't be drawn (shown in the dialog and logged, never silent: #145).
    error: Option<String>,
    canvas_proxy: Option<(u32, Arc<Document>)>,
    overlay: Option<egui::TextureHandle>,
    overlay_key: u64,
}

fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

fn query_key(app: &PhotocraftApp, params: &Value) -> u64 {
    let mut key = std::collections::hash_map::DefaultHasher::new();
    params.to_string().hash(&mut key);
    let foreground = app.session.tools.foreground;
    foreground.map(f32::to_bits).hash(&mut key);
    // This runs several times per frame, including mere pointer motion. Debug-formatting a
    // ProofView serializes the whole ICC profile/LUT; proof() also clones the default profile.
    // Borrow the state and use the profile's memoized content hash instead.
    let proof = app.session.active().and_then(|st| app.session.color.proof_ref(st.doc.id));
    proof.is_some().hash(&mut key);
    if let Some(proof) = proof {
        let setup = &proof.setup;
        (setup.profile.content_hash(), setup.intent, setup.bpc, setup.simulate_paper, setup.kind).hash(&mut key);
        (proof.enabled, proof.gamut_warning, proof.gamut_threshold.to_bits()).hash(&mut key);
    }
    key.finish().max(1)
}

/// Keep the accelerated preview paths in sync with the engine's Color Range replacement.
/// The full-resolution engine handles other combine modes itself.
fn scope_preview(proxy: &Document, params: &Value, mask: &mut [f32]) {
    if matches!(params.get("mode").and_then(Value::as_str), Some("add" | "subtract" | "intersect")) {
        return;
    }
    if let Some(selection) = proxy.selection.as_ref() {
        let prior = photocraft_algo::selection::mask_from_surface(Some(selection), proxy.bounds());
        for (value, bound) in mask.iter_mut().zip(prior) {
            *value = value.min(bound);
        }
    }
}

/// The selection mask `params` would make, on the proxy: the engine command run on a scratch
/// session (Out Of Gamut uses the document's own proof setup).
fn proxy_mask(app: &PhotocraftApp, proxy: &Document, k: u32, params: &Value) -> Result<Vec<f32>, String> {
    let area = proxy.bounds();
    if params.get("select").and_then(Value::as_str).unwrap_or("sampledColors") == "sampledColors" {
        // Pick on the original document, not the thumbnail's nearest retained pixel. The
        // prepared Lab query is the same one the command uses at full resolution.
        let query = photocraft_engine::selection_cmds::ColorRangeSamples::new(&app.session, params).map_err(|e| e.to_string())?;
        // Like the command (and OK): `sampleAllLayers: false` judges the active layer's own
        // pixels, otherwise the composite.
        let all_layers = params.get("sampleAllLayers").and_then(Value::as_bool).unwrap_or(true);
        let layer = app.session.active().and_then(|d| d.active_layer).and_then(|id| proxy.layer(id)).and_then(|l| l.surface());
        let px = match (all_layers, layer) {
            (false, Some(surf)) => {
                let mut px = vec![[0.0f32; 4]; area.width() as usize * area.height() as usize];
                surf.read_rgba_into(area, &mut px);
                px
            }
            _ => photocraft_compose::render(proxy, area).px,
        };
        let invert = params.get("invert").and_then(Value::as_bool).unwrap_or(false);
        let mut mask: Vec<f32> = query
            .coverage(&px, area.width() as usize, k as f32, [0.0, 0.0])
            .into_iter()
            .map(|v| {
                let v = if invert { 1.0 - v } else { v };
                // Match the command's GRAY8 mask rounding, before display interpolation.
                (v.clamp(0.0, 1.0) * 255.0 + 0.5).floor() / 255.0
            })
            .collect();
        scope_preview(proxy, params, &mut mask);
        return Ok(mask);
    }
    if params.get("select").and_then(Value::as_str) == Some("outOfGamut") {
        let pv = app.session.color.proof(proxy.id);
        let (m, _) = photocraft_engine::color_cmds::gamut_mask(proxy, &pv.setup, pv.gamut_threshold).map_err(|e| e.to_string())?;
        let invert = params.get("invert").and_then(Value::as_bool).unwrap_or(false);
        let mut mask: Vec<f32> = m.iter().map(|v| f32::from(*v) / 255.0).map(|v| if invert { 1.0 - v } else { v }).collect();
        scope_preview(proxy, params, &mut mask);
        return Ok(mask);
    }
    let mut p = params.clone();
    // Eyedropper points are in document pixels; the proxy is 1/k the size.
    let (w, h) = (f64::from(area.width().max(1)), f64::from(area.height().max(1)));
    if let Some(o) = p.as_object_mut() {
        for key in ["points", "subtractPoints"] {
            if let Some(Value::Array(a)) = o.get_mut(key) {
                for v in a.iter_mut() {
                    if let Some([x, y]) = v.as_array().map(Vec::as_slice).and_then(|s| match s {
                        [x, y] => Some([x.as_f64()?, y.as_f64()?]),
                        _ => None,
                    }) {
                        *v = json!([(x / f64::from(k)).floor().clamp(0.0, w - 1.0), (y / f64::from(k)).floor().clamp(0.0, h - 1.0)]);
                    }
                }
            }
        }
    }
    let mut s = photocraft_engine::Session::new();
    s.tools = app.session.tools.clone();
    // Preserve the proxy's pre-existing selection so preview and OK use the same scope.
    s.add_document(proxy.clone(), None);
    if let Some(id) = app.session.active().and_then(|d| d.active_layer) {
        let _ = s.select_layer(id);
    }
    s.execute(COMMAND, p).map_err(|e| e.to_string())?;
    let d = s.active().ok_or("no preview document")?;
    Ok(photocraft_algo::selection::mask_from_surface(d.doc.selection.as_ref(), area))
}

/// Record why the preview failed (logged once per new reason), or clear it.
fn report(app: &mut PhotocraftApp, error: Option<String>) {
    let Some(p) = app.color_range.as_mut() else { return };
    if p.error != error {
        if let Some(e) = &error {
            log::warn!("Color Range preview: {e}");
        }
        p.error = error;
    }
}

/// Refresh the cached proxy / textures for the active document and the dialog's params. Returns
/// (texture to show, its size in proxy pixels, k).
fn preview(app: &mut PhotocraftApp, ctx: &egui::Context, f: &Map<String, Value>) -> Option<(egui::TextureId, [usize; 2], u32)> {
    let (doc_id, revision, doc) = {
        let st = app.session.active()?;
        (st.doc.id, st.revision, st.doc.clone())
    };
    if !matches!(&app.color_range, Some(p) if p.doc == doc_id && p.revision == revision) {
        let side = doc.size.width.max(doc.size.height).max(1);
        let k = side.div_ceil(PREVIEW).max(1);
        let proxy = Arc::new(crate::proxy::proxy_document(&doc, k));
        app.color_range = Some(Preview {
            doc: doc_id,
            revision,
            k,
            proxy,
            image: None,
            mask_key: 0,
            mask: None,
            error: None,
            canvas_proxy: None,
            overlay: None,
            overlay_key: 0,
        });
    }
    let image_view = (s(f, "__view", "selection") == "image") ^ ctx.input(|i| i.modifiers.command || (!cfg!(target_os = "macos") && i.modifiers.ctrl));
    let params = params(f);
    let key = query_key(app, &params);
    let (proxy, k) = {
        let p = app.color_range.as_ref()?;
        (p.proxy.clone(), p.k)
    };
    let (w, h) = (proxy.size.width as usize, proxy.size.height as usize);
    if image_view {
        if app.color_range.as_ref().is_some_and(|p| p.image.is_none()) {
            let buf = photocraft_compose::thumbnail_buffer(&proxy, proxy.size.width.max(proxy.size.height));
            let thumb = match app.session.color.canvas_display(&proxy) {
                Ok(display) => display.to_rgba8(&buf),
                Err(e) => {
                    report(app, Some(e.to_string()));
                    return None;
                }
            };
            let size = [thumb.width as usize, thumb.height as usize];
            if size != [w, h] || thumb.pixels.len() != w * h * 4 {
                report(app, Some(format!("the image thumbnail is {}×{}, expected {w}×{h}", thumb.width, thumb.height)));
                return None;
            }
            let img = egui::ColorImage::from_rgba_unmultiplied(size, &thumb.pixels);
            let tex = ctx.load_texture("color-range-image", img, egui::TextureOptions::LINEAR);
            if let Some(p) = app.color_range.as_mut() {
                p.image = Some(tex);
            }
        }
        return app.color_range.as_ref()?.image.as_ref().map(|t| (t.id(), [w, h], k));
    }
    if app.color_range.as_ref().is_some_and(|p| p.mask_key != key || p.mask.is_none()) {
        let mask = match proxy_mask(app, &proxy, k, &params) {
            Ok(m) => {
                report(app, None);
                m
            }
            Err(e) => {
                report(app, Some(e));
                vec![0.0; w * h]
            }
        };
        if mask.len() != w * h {
            report(app, Some(format!("the selection preview has {} pixels, expected {}", mask.len(), w * h)));
            return None;
        }
        let px: Vec<Color32> = mask.iter().map(|v| Color32::from_gray((v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect();
        let img = egui::ColorImage::new([w, h], px);
        let p = app.color_range.as_mut()?;
        match &mut p.mask {
            Some(t) => t.set(img, egui::TextureOptions::LINEAR),
            None => p.mask = Some(ctx.load_texture("color-range-mask", img, egui::TextureOptions::LINEAR)),
        }
        p.mask_key = key;
    }
    app.color_range.as_ref()?.mask.as_ref().map(|t| (t.id(), [w, h], k))
}

/// Photoshop-style radio button.
fn radio(ui: &mut egui::Ui, on: bool, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (r, resp) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::click());
        ui.painter().circle_stroke(r.center(), 5.5, Stroke::new(1.2, if on { t.accent } else { t.text_faint }));
        if on {
            ui.painter().circle_filled(r.center(), 3.0, t.accent);
        }
        let l = ui
            .add(egui::Label::new(egui::RichText::new(tl!(&label)).color(t.text_dim)).selectable(false).sense(Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        resp.union(l)
    })
    .inner
}

/// Eyedropper button: the pipette icon with a +/− badge; labelled for accessibility.
fn eyedropper(ui: &mut egui::Ui, badge: &str, selected: bool, label: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let r = crate::icons::button(ui, "pipette", 26.0, selected, label);
    if !badge.is_empty() {
        let at = r.rect.right_bottom() - vec2(5.0, 6.0);
        ui.painter().text(at, egui::Align2::CENTER_CENTER, badge, crate::theme::semibold(12.0), if selected { t.accent_text } else { t.text });
    }
    r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label));
    r
}

fn set_points(f: &mut Map<String, Value>, k: &str, pts: &[[f64; 2]]) {
    f.insert(k.into(), json!(pts));
}

/// Dialog body.
pub fn body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    let c = controls(f);
    let select = s(f, "select", "sampledColors").to_string();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Select:")).color(t.text_dim));
        let mut cur: &str = &select;
        if widgets::dropdown(ui, "color-range-select", tl!("Select"), &mut cur, SELECTS, 170.0) {
            f.insert("select".into(), json!(cur));
        }
    });
    ui.add_space(4.0);
    ui.add_enabled_ui(c.sampling, |ui| {
        let mut on = flag(f, "localized");
        if widgets::checkbox(ui, &mut on, tl!("Localized Color Clusters")).changed() {
            f.insert("localized".into(), json!(on));
        }
    });
    ui.add_space(4.0);
    ui.add_enabled_ui(c.fuzziness, |ui| {
        if c.tonal {
            let mut v = num(f, "toneFuzziness", 20.0);
            if widgets::slider_row(ui, tl!("Fuzziness:"), &mut v, 0.0..=100.0, "%", None).changed() {
                f.insert("toneFuzziness".into(), json!(v.round()));
            }
        } else {
            let mut v = num(f, "fuzziness", 40.0);
            if widgets::slider_row(ui, tl!("Fuzziness:"), &mut v, 0.0..=200.0, "", None).changed() {
                f.insert("fuzziness".into(), json!(v.round()));
            }
        }
    });
    if c.range {
        let mut v = num(f, "range", 100.0);
        if widgets::slider_row(ui, tl!("Range:"), &mut v, 0.0..=100.0, "%", None).changed() {
            f.insert("range".into(), json!(v.round()));
        }
    }
    if c.tonal {
        let mut level = |ui: &mut egui::Ui, key: &str, label: &str, d: f32| {
            let mut v = num(f, key, d);
            if widgets::slider_row(ui, label, &mut v, 0.0..=255.0, "", None).changed() {
                f.insert(key.into(), json!(v.round()));
            }
        };
        match select.as_str() {
            "shadows" => level(ui, "shadowsLevel", "Shadows up to:", 65.0),
            "highlights" => level(ui, "highlightsLevel", "Highlights from:", 190.0),
            _ => {
                level(ui, "midtonesLow", tl!("Midtones from:"), 105.0);
                level(ui, "midtonesHigh", tl!("Midtones to:"), 150.0);
            }
        }
    }
    ui.add_space(6.0);
    ui.horizontal_top(|ui| {
        // Preview: the selection as a grayscale mask, or the image to pick colours from.
        let box_size = vec2(PREVIEW as f32, PREVIEW as f32);
        let (frame, resp) = ui.allocate_exact_size(box_size, Sense::click_and_drag());
        ui.painter().rect_filled(frame, 0.0, t.canvas);
        let shown = preview(app, ui.ctx(), f);
        if let Some((tex, [w, h], k)) = shown {
            let scale = (box_size.x / w.max(1) as f32).min(box_size.y / h.max(1) as f32);
            let img_rect = Rect::from_center_size(frame.center(), vec2(w as f32 * scale, h as f32 * scale));
            ui.painter().image(tex, img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            // Eyedropper on the preview (Photoshop also samples in the preview area).
            // Response::clicked may consume accessibility input (a write lock): query it
            // before reading PointerState, never from inside Ui::input's read lock.
            let clicked = resp.clicked();
            let held = resp.is_pointer_button_down_on();
            let sample =
                ui.input(|i| (clicked && i.pointer.primary_pressed()) || (held && (i.pointer.primary_pressed() || i.pointer.delta() != egui::Vec2::ZERO)));
            if let Some(p) = resp.interact_pointer_pos().filter(|p| c.sampling && sample && img_rect.contains(*p)) {
                let at = [
                    ((p.x - img_rect.left()) / scale).floor().max(0.0) as f64 * f64::from(k) + f64::from(k / 2),
                    ((p.y - img_rect.top()) / scale).floor().max(0.0) as f64 * f64::from(k) + f64::from(k / 2),
                ];
                pick(app, f, at, ui.input(|i| i.modifiers));
            }
        }
        // A preview that can't be drawn says why (#145) instead of staying blank.
        if let Some(e) = app.color_range.as_ref().and_then(|p| p.error.clone()) {
            ui.painter().rect_filled(frame, 0.0, t.canvas);
            let msg = format!(
                "Preview unavailable:
{e}"
            );
            let g = ui.painter().layout(msg, egui::FontId::proportional(11.0), t.text_faint, frame.width() - 16.0);
            ui.painter().galley(frame.center() - g.size() / 2.0, g, t.text_faint);
        }
        ui.painter().rect_stroke(frame, 0.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
        let label = match app.color_range.as_ref().and_then(|p| p.error.as_deref()) {
            Some(e) => format!("Color Range preview unavailable: {e}"),
            None => tl!("Color Range preview").to_string(),
        };
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, ui.is_enabled(), &label));
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.add_enabled_ui(c.sampling, |ui| {
                let tool = s(f, "__tool", "sample").to_string();
                ui.horizontal(|ui| {
                    for (id, badge, label) in
                        [("sample", "", tl!("Eyedropper")), ("add", "+", tl!("Add to Sample")), ("subtract", "−", tl!("Subtract from Sample"))]
                    {
                        if eyedropper(ui, badge, tool == id, label).clicked() {
                            f.insert("__tool".into(), json!(id));
                        }
                    }
                });
                let (add, sub) = (points(f, "points").len(), points(f, "subtractPoints").len());
                let what = match (add, sub) {
                    (0, 0) => tl!("Sample: foreground colour").to_string(),
                    (a, 0) => crate::i18n::trn(crate::i18n::current(), a as u64, "{n} sample", "{n} samples"),
                    (a, s) => crate::i18n::fmt(
                        tl!("{added}, {removed} subtracted"),
                        &[("added", &crate::i18n::trn(crate::i18n::current(), a as u64, "{n} sample", "{n} samples")), ("removed", &s.to_string())],
                    ),
                };
                ui.label(egui::RichText::new(what).size(11.0).color(t.text_faint));
                if add + sub > 0 && widgets::secondary_button(ui, tl!("Clear Samples"), 0.0).clicked() {
                    set_points(f, "points", &[]);
                    set_points(f, "subtractPoints", &[]);
                    f.insert("__order".into(), json!([]));
                }
            });
            ui.add_space(8.0);
            let mut inv = flag(f, "invert");
            // Selection invert (≠ Image › Adjustments › Invert → Đảo màu).
            if widgets::checkbox(ui, &mut inv, crate::i18n::tr_ctx(crate::i18n::current(), "colorRange", "Invert")).changed() {
                f.insert("invert".into(), json!(inv));
            }
        });
    });
    ui.add_space(4.0);
    let view = s(f, "__view", "selection").to_string();
    ui.horizontal(|ui| {
        if radio(ui, view == "selection", "Selection").clicked() {
            f.insert("__view".into(), json!("selection"));
        }
        ui.add_space(8.0);
        if radio(ui, view == "image", "Image").clicked() {
            f.insert("__view".into(), json!("image"));
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Selection Preview:")).color(t.text_dim));
        let mode = s(f, "__selectionPreview", "none").to_string();
        let mut chosen = mode.as_str();
        if widgets::dropdown(ui, "color-range-selection-preview", tl!("Preview"), &mut chosen, PREVIEWS, 170.0) {
            f.insert("__selectionPreview".into(), json!(chosen));
        }
    });
    if app.session.active().is_none() {
        ui.label(egui::RichText::new(tl!("Open a document to select a colour range.")).color(t.text_faint));
    }
}

/// A selection view is an overlay only: eyedroppers always sample the original image below it.
/// White = selected, black = unselected, gray = partial; matte/Quick Mask cover unselected pixels.
fn overlay_pixel(mode: &str, coverage: f32) -> Color32 {
    let coverage = coverage.clamp(0.0, 1.0);
    let q = |v: f32| (v * 255.0 + 0.5) as u8;
    match mode {
        "grayscale" => Color32::from_gray(q(coverage)),
        "blackMatte" => Color32::from_black_alpha(q(1.0 - coverage)),
        "whiteMatte" => Color32::from_rgba_unmultiplied(255, 255, 255, q(1.0 - coverage)),
        "quickMask" => Color32::from_rgba_unmultiplied(255, 0, 0, q(0.5 * (1.0 - coverage))),
        _ => Color32::TRANSPARENT,
    }
}

/// Photoshop's Selection Preview on the image, shared by CPU/GPU canvases and all platforms.
/// This never edits the real selection, image, channels or history; closing the dialog removes it.
pub(crate) fn paint_selection_preview(app: &mut PhotocraftApp, ctx: &egui::Context, painter: &egui::Painter, doc: DocId, image: Rect, flip: bool) {
    let Some(fields) = app.ui.dialogs.last().filter(|d| owns(&d.fields)).map(|d| d.fields.clone()) else { return };
    let mode = s(&fields, "__selectionPreview", "none");
    if mode == "none" || !app.session.active().is_some_and(|st| st.doc.id == doc) {
        return;
    }
    // Refresh the document/revision cache before borrowing its canvas proxy.
    let _ = preview(app, ctx, &fields);
    if app.color_range.as_ref().is_some_and(|p| p.canvas_proxy.is_none()) {
        let Some(original) = app.session.active().map(|st| st.doc.clone()) else { return };
        let k = original.size.width.max(original.size.height).max(1).div_ceil(CANVAS_PREVIEW);
        let proxy = Arc::new(crate::proxy::proxy_document(&original, k));
        if let Some(p) = app.color_range.as_mut() {
            p.canvas_proxy = Some((k, proxy));
        }
    }
    let params = params(&fields);
    let key = (query_key(app, &params) ^ hash(mode)).max(1);
    if app.color_range.as_ref().is_some_and(|p| p.overlay_key != key || p.overlay.is_none()) {
        let Some((k, proxy)) = app.color_range.as_ref().and_then(|p| p.canvas_proxy.clone()) else { return };
        let mask = match proxy_mask(app, &proxy, k, &params) {
            Ok(mask) => mask,
            Err(e) => {
                report(app, Some(e));
                return;
            }
        };
        let size = [proxy.size.width as usize, proxy.size.height as usize];
        let pixels = mask.into_iter().map(|v| overlay_pixel(mode, v)).collect();
        let img = egui::ColorImage::new(size, pixels);
        if let Some(p) = app.color_range.as_mut() {
            match &mut p.overlay {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => p.overlay = Some(ctx.load_texture("color-range-canvas-preview", img, egui::TextureOptions::LINEAR)),
            }
            p.overlay_key = key;
        }
    }
    if let Some(tex) = app.color_range.as_ref().and_then(|p| p.overlay.as_ref()) {
        let uv = if flip { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)) };
        painter.image(tex.id(), image, uv, Color32::WHITE);
    }
}

/// The top dialog's fields when it is Color Range in a mode that takes eyedropper samples.
fn sampling_top(app: &PhotocraftApp) -> Option<Map<String, Value>> {
    app.ui.dialogs.last().map(|d| &d.fields).filter(|f| owns(f) && controls(f).sampling).cloned()
}

/// The dialog's eyedropper on the canvas: with Color Range › Sampled Colors as the top dialog,
/// picks the colour at document point `at` (Shift adds to the sample, Alt subtracts), as a click
/// on the image does in Photoshop. Returns whether it picked.
pub fn pick_top(app: &mut PhotocraftApp, at: [f64; 2], mods: egui::Modifiers) -> bool {
    let Some(mut f) = sampling_top(app) else { return false };
    let Some(doc) = app.session.active().map(|st| &st.doc) else { return false };
    if !at[0].is_finite() || !at[1].is_finite() || at[0] < 0.0 || at[1] < 0.0 || at[0] >= f64::from(doc.size.width) || at[1] >= f64::from(doc.size.height) {
        return false;
    }
    pick(app, &mut f, at, mods);
    match app.ui.dialogs.last_mut() {
        Some(d) => {
            d.fields = f;
            true
        }
        None => false,
    }
}

/// The canvas under an open Color Range dialog is its eyedropper (Photoshop): the pointer `over`
/// shows the dialog's eyedropper (+ / − while Shift / Alt add or subtract), and a press at document
/// point `press` takes a sample. False when Color Range isn't the top dialog.
pub fn canvas_eyedropper(app: &mut PhotocraftApp, ctx: &egui::Context, over: egui::Pos2, press: Option<[f64; 2]>) -> bool {
    let Some(f) = sampling_top(app) else { return false };
    let mods = ctx.input(|i| i.modifiers);
    let tool = if mods.shift {
        "add"
    } else if mods.alt {
        "subtract"
    } else {
        s(&f, "__tool", "sample")
    };
    if app.session.prefs().cursors.other == photocraft_engine::prefs::OtherCursor::Precise {
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
    } else {
        // The tip of the icon's pipette is at (2, 22) of its 24-unit box (as the Color Picker's).
        crate::icons::cursor(ctx, "pipette", over, vec2(2.0, 22.0) / 24.0, 20.0);
        ctx.set_cursor_icon(egui::CursorIcon::None);
    }
    let badge = match tool {
        "add" => "+",
        "subtract" => "−",
        _ => "",
    };
    if !badge.is_empty() {
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("pc-color-range-badge")));
        let at = over + vec2(20.0, -4.0);
        p.text(at + vec2(1.0, 1.0), egui::Align2::CENTER_CENTER, badge, crate::theme::semibold(13.0), Color32::from_black_alpha(200));
        p.text(at, egui::Align2::CENTER_CENTER, badge, crate::theme::semibold(13.0), Color32::WHITE);
    }
    if let Some(at) = press
        && pick_top(app, at, mods)
    {
        ctx.request_repaint();
    }
    true
}

/// An eyedropper click at document pixel `at`: the plain eyedropper replaces the samples, the
/// + one (or Shift) adds one, the − one (or Alt) subtracts one.
pub fn pick(app: &PhotocraftApp, f: &mut Map<String, Value>, at: [f64; 2], mods: egui::Modifiers) {
    let Some(st) = app.session.active() else { return };
    let (w, h) = (f64::from(st.doc.size.width.max(1)), f64::from(st.doc.size.height.max(1)));
    let [x, y] = at;
    let at = [x.floor().clamp(0.0, w - 1.0), y.floor().clamp(0.0, h - 1.0)];
    let tool = if mods.shift {
        "add"
    } else if mods.alt {
        "subtract"
    } else {
        s(f, "__tool", "sample")
    };
    // The click order matters: Photoshop applies additions and subtractions one after another.
    let mut order: Vec<Value> = f.get("__order").and_then(Value::as_array).cloned().unwrap_or_default();
    match tool {
        "add" => {
            let mut p = points(f, "points");
            p.push(at);
            set_points(f, "points", &p);
            order.push(json!("+"));
        }
        "subtract" => {
            let mut p = points(f, "subtractPoints");
            p.push(at);
            set_points(f, "subtractPoints", &p);
            order.push(json!("-"));
        }
        _ => {
            set_points(f, "points", &[at]);
            set_points(f, "subtractPoints", &[]);
            order = vec![json!("+")];
        }
    }
    f.insert("__order".into(), Value::Array(order));
}

/// The click order for the command (`order`), when it is consistent with the point lists.
fn order(f: &Map<String, Value>, add: usize, sub: usize) -> Option<Value> {
    let o = f.get("__order").and_then(Value::as_array)?;
    let plus = o.iter().filter(|v| v.as_str() == Some("+")).count();
    let minus = o.iter().filter(|v| v.as_str() == Some("-")).count();
    (plus == add && minus == sub && plus + minus == o.len()).then(|| Value::Array(o.clone()))
}

#[cfg(test)]
mod tests;
