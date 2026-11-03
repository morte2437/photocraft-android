//! Live canvas preview for hovered fonts in the Type tool's font picker (#2315).

use std::collections::VecDeque;
use std::sync::Arc;

use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;

use crate::PhotocraftApp;

const TAG: u64 = 1 << 60;

/// How long the pointer has to rest on a served family that isn't fetched yet before hovering it
/// fetches it (seconds). Hovering asks the host for the whole family, and sweeping the font menu
/// would otherwise download every family the pointer passes; installed fonts preview at once.
const HOVER_FETCH_DELAY: f64 = 0.25;

#[derive(Clone)]
pub struct TypeFontPreview {
    document: DocId,
    revision: u64,
    layer: LayerId,
    range: Option<[usize; 2]>,
    family: Option<String>,
    shown: Option<Arc<Document>>,
    key: u64,
    original_area: Rect,
    areas: VecDeque<(u64, Rect)>,
}

pub(crate) fn follow_hover(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let hover_id = egui::Id::unique("type-font-hover");
    let since_id = egui::Id::unique("type-font-hover-since");
    let family = ctx.data_mut(|d| {
        let family = d.get_temp::<String>(hover_id);
        d.remove::<String>(hover_id);
        family
    });
    if let Some(family) = family {
        let now = ctx.input(|i| i.time);
        let since = ctx.data_mut(|d| match d.get_temp::<(String, f64)>(since_id) {
            Some((f, t)) if f == family => t,
            _ => {
                d.insert_temp(since_id, (family.clone(), now));
                now
            }
        });
        let installed = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).fonts.has_family(&family);
        let rested = now - since;
        if installed || rested >= HOVER_FETCH_DELAY {
            set(app, family);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(HOVER_FETCH_DELAY - rested));
        }
    } else {
        ctx.data_mut(|d| d.remove::<(String, f64)>(since_id));
        if app.type_font_preview.is_some() {
            clear(app);
        }
    }
}

pub(crate) fn display_doc(app: &mut PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    let p = app.type_font_preview.as_ref()?.clone();
    let family = p.family.as_deref()?;
    let st = app.session.documents().get(idx)?;
    if st.doc.id != p.document || st.revision != p.revision {
        return None;
    }
    if !photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner()).fonts.has_family(family) {
        return None;
    }
    if let Some(shown) = &p.shown {
        return Some((shown.clone(), p.key));
    }

    // Clone the original document to avoid mutating the session's document
    let original_doc = st.doc.clone();
    let mut doc = (*st.doc).clone();
    let layer = p.layer;
    let family = family.to_string();
    let range = p.range;

    {
        let l = doc.layer_mut(layer)?;
        if let LayerContent::Text(t) = &mut l.content {
            let (start, end) = range.map_or((0, t.text.len()), |[a, b]| (photocraft_text::byte_index(&t.text, a), photocraft_text::byte_index(&t.text, b)));
            photocraft_engine::type_cmds::style_range(t, start, end, &|style| {
                style.font_family.clone_from(&family);
                style.postscript_name = None;
            });
            photocraft_engine::type_cmds::refresh(&original_doc, t);
        } else {
            return None;
        }
    }

    let area = doc.layer(layer)?.surface().map_or(Rect::EMPTY, |surface| surface.tile_bounds());
    let shown = Arc::new(photocraft_engine::mode_cmds::display_document(&doc).unwrap_or(doc));
    let key = p.key;
    if let Some(p) = app.type_font_preview.as_mut() {
        if p.areas.len() >= 128 {
            p.areas.pop_front();
        }
        p.areas.push_back((key, area));
        p.shown = Some(shown.clone());
    }
    Some((shown, key))
}

pub(crate) fn set(app: &mut PhotocraftApp, family: String) {
    // A caret's font formats subsequent typing, not the existing layer's pixels.
    if app.ui.text_edit.as_ref().is_some_and(|ed| ed.caret == ed.anchor) {
        clear(app);
        return;
    }
    let Some((layer, range)) = crate::type_tool::target(app) else {
        clear(app);
        return;
    };
    let layer = LayerId(layer);
    let Some(st) = app.session.active() else {
        clear(app);
        return;
    };
    if !matches!(st.doc.layer(layer).map(|l| &l.content), Some(LayerContent::Text(_))) {
        clear(app);
        return;
    }
    photocraft_text::served::request(&family);
    if app.type_font_preview.as_ref().is_some_and(|p| {
        p.family.as_deref() == Some(family.as_str()) && p.document == st.doc.id && p.revision == st.revision && p.layer == layer && p.range == range
    }) {
        return;
    }
    let doc_id = st.doc.id;
    let revision = st.revision;
    let original_area = st.doc.layer(layer).and_then(|l| l.surface()).map_or(Rect::EMPTY, |s| s.tile_bounds());
    let previous = app.type_font_preview.take().filter(|p| p.document == doc_id && p.revision == revision && p.layer == layer && p.range == range);
    let (areas, original_area) = previous.map_or((VecDeque::new(), original_area), |p| (p.areas, p.original_area));
    let key = preview_key(revision, layer, range, &family);
    app.type_font_preview = Some(TypeFontPreview { document: doc_id, revision, layer, range, family: Some(family), shown: None, key, original_area, areas });
}

pub(crate) fn clear(app: &mut PhotocraftApp) {
    if let Some(p) = app.type_font_preview.as_mut() {
        p.family = None;
        p.shown = None;
    }
}

pub(crate) fn damage(app: &PhotocraftApp, doc: DocId, revision: u64, before: u64, after: u64) -> Option<Rect> {
    if before == after {
        return None;
    }
    let p = app.type_font_preview.as_ref().filter(|p| p.document == doc && p.revision == revision)?;
    let area = |key| if key == 0 { Some(p.original_area) } else { p.areas.iter().find(|(k, _)| *k == key).map(|(_, r)| *r) };
    Some(area(before)?.union(&area(after)?))
}

fn preview_key(revision: u64, layer: LayerId, range: Option<[usize; 2]>, family: &str) -> u64 {
    let range_key = range.map_or(0, |[start, end]| (start as u64).rotate_left(17) ^ end as u64);
    let h = family.bytes().fold(revision.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ layer.0 ^ range_key, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
    (h & (TAG - 1)) | TAG
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Services;
    use serde_json::json;

    /// One frame at `time` (seconds) with the pointer on `family`'s row in the font menu (`None`:
    /// on no row), as the menu reports it each frame.
    fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, family: Option<&str>, time: f64) {
        let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
            if let Some(f) = family {
                ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("type-font-hover"), f.to_string()));
            }
            follow_hover(app, ui.ctx());
        });
        out.textures_delta.clear();
    }

    fn previewed(app: &PhotocraftApp) -> Option<&str> {
        app.type_font_preview.as_ref().and_then(|p| p.family.as_deref())
    }

    fn app_with_type_layer() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
        app
    }

    #[test]
    fn a_served_family_is_asked_for_only_once_the_pointer_rests_on_its_row() {
        // Families the host serves but hasn't sent (own names: the served catalog is process-wide).
        let (a, b) = ("Served Hover Rest A", "Served Hover Rest B");
        let (fonts, _) = photocraft_text::served::parse_manifest(&format!("{a} | Regular | fonts/a/A.ttf\n{b} | Regular | fonts/b/B.ttf\n"));
        photocraft_text::served::add_fonts(&fonts);
        let mut app = app_with_type_layer();
        let ctx = egui::Context::default();

        // Passing over rows asks for nothing: each new row starts the wait again.
        frame(&mut app, &ctx, Some(a), 1.0);
        frame(&mut app, &ctx, Some(a), 1.0 + HOVER_FETCH_DELAY / 2.0);
        frame(&mut app, &ctx, Some(b), 1.0 + HOVER_FETCH_DELAY);
        frame(&mut app, &ctx, Some(a), 1.0 + HOVER_FETCH_DELAY * 1.5);
        assert_eq!(previewed(&app), None);

        // Resting on a row asks for the family and previews it.
        frame(&mut app, &ctx, Some(a), 1.0 + HOVER_FETCH_DELAY * 3.0);
        assert_eq!(previewed(&app), Some(a));

        // Leaving the menu clears the preview, and the next hover waits again.
        frame(&mut app, &ctx, None, 3.0);
        assert_eq!(previewed(&app), None);
        frame(&mut app, &ctx, Some(a), 3.1);
        assert_eq!(previewed(&app), None);
    }

    #[test]
    fn an_installed_family_previews_at_once() {
        let mut app = app_with_type_layer();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, Some("JetBrains Mono"), 1.0);
        assert_eq!(previewed(&app), Some("JetBrains Mono"));
    }
}
