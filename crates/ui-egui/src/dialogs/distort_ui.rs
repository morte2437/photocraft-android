//! Interactive Liquify (full-window dialog), Puppet Warp, Perspective Warp and Vanishing Point
//! (on-canvas modes).
//!
//! All three are previews over plain data (strokes, pins, planes); committing runs the engine
//! command with that data (`filter.liquify`, `edit.puppetWarp`, `edit.perspectiveWarp`,
//! `filter.vanishingPoint`, or the smart-object twins), so the result is one history step and exactly replayable. This module
//! holds the shared runtime state and the hooks the shell calls (menus, pointer, keys, overlay,
//! options bar); the modes live in `liquify_ui`, `puppet_ui`, `perspective_ui` and `vp_ui`.

use std::sync::Arc;

use egui::{Color32, ColorImage};
use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};

/// Runtime state of the distortion modes (not serialisable: textures, solvers).
#[derive(Default)]
pub struct Distort {
    pub liquify: Option<crate::liquify_ui::LiquifyDialog>,
    /// Edit › Content-Aware Fill… (caf_ui).
    pub caf: Option<crate::caf_ui::CafWorkspace>,
    /// Select › Select and Mask… (select_mask_ui).
    pub select_mask: Option<crate::select_mask_ui::SelectMaskWorkspace>,
    pub puppet: Option<crate::puppet_ui::PuppetSession>,
    pub perspective: Option<crate::perspective_ui::PerspSession>,
    pub vanishing: Option<crate::vp_ui::VpSession>,
    /// Filter Gallery dialog (gallery_ui).
    pub gallery: Option<crate::gallery_ui::GalleryDialog>,
    /// The document the open sessions belong to, noted by [`drop_foreign`] on the first frame they are open.
    owner: Option<photocraft_doc::DocId>,
}

impl Distort {
    pub fn active(&self) -> bool {
        self.liquify.is_some()
            || self.caf.is_some()
            || self.select_mask.is_some()
            || self.puppet.is_some()
            || self.perspective.is_some()
            || self.vanishing.is_some()
    }

    /// Short description for `ui.inspect`.
    pub fn describe(&self) -> Value {
        json!({
            "liquify": self.liquify.as_ref().map(|d| d.describe()),
            "contentAwareFill": self.caf.as_ref().map(|d| d.describe()),
            "selectAndMask": self.select_mask.as_ref().map(|d| d.describe()),
            "puppet": self.puppet.as_ref().map(|p| p.describe()),
            "perspective": self.perspective.as_ref().map(|p| p.describe()),
            "vanishingPoint": self.vanishing.as_ref().map(|v| v.describe()),
            "gallery": self.gallery.as_ref().map(|g| g.describe()),
        })
    }
}

/// The active layer's pixels (a pixel layer, or a smart object's rendering) and its id.
pub(crate) fn active_pixels(app: &PhotocraftApp) -> Result<(LayerId, Surface, bool), String> {
    let st = app.session.active().ok_or("no document")?;
    let id = st.active_layer.ok_or("no active layer")?;
    let l = st.doc.layer(id).ok_or("no layer")?;
    match &l.content {
        LayerContent::Raster(s) => Ok((id, s.clone(), false)),
        LayerContent::Smart(sm) => sm.cache.clone().map(|c| (id, c, true)).ok_or_else(|| "the smart object has no pixels".to_string()),
        other => Err(format!("needs a pixel layer or smart object (active layer is {} {} layer)", other.article(), other.kind_name())),
    }
}

/// The document with layer `id` hidden (what shows behind a live preview of that layer).
pub(crate) fn without_layer(doc: &Document, id: LayerId) -> Arc<Document> {
    let mut d = doc.clone();
    if let Some(l) = d.layer_mut(id) {
        l.visible = false;
    }
    Arc::new(d)
}

/// A texture image of `surf` over `r`, downsampled so the longer side is at most `max_side`.
pub(crate) fn surface_image(surf: &Surface, r: Rect, max_side: usize) -> ColorImage {
    let (w, h) = (r.width().max(1) as usize, r.height().max(1) as usize);
    let k = w.max(h).div_ceil(max_side).max(1);
    let (tw, th) = (w.div_ceil(k), h.div_ceil(k));
    let mut px = vec![Color32::TRANSPARENT; tw * th];
    let mut row = vec![[0u8; 4]; w];
    for ty in 0..th {
        let y = r.y0 + (ty * k) as i32;
        surf.read_rgba8_into(Rect::new(r.x0, y, r.x1, y + 1), &mut row);
        for tx in 0..tw {
            let p = row[(tx * k).min(w - 1)];
            let a = if surf.format().alpha { p[3] } else { 255 };
            px[ty * tw + tx] = Color32::from_rgba_unmultiplied(p[0], p[1], p[2], a);
        }
    }
    ColorImage::new([tw, th], px)
}

/// The menu id of the open full-window workspace (Liquify, Content-Aware Fill, Filter Gallery,
/// Adaptive Wide Angle). These are modal: other menu commands wait for OK or Cancel
/// (`menus::modal_allows`).
pub(crate) fn modal_workspace(app: &PhotocraftApp) -> Option<&'static str> {
    if app.distort.select_mask.is_some() {
        Some("select.selectAndMask")
    } else if app.distort.liquify.is_some() {
        Some("filter.liquify")
    } else if app.distort.caf.is_some() {
        Some("edit.contentAwareFill")
    } else if app.distort.gallery.is_some() {
        Some("filter.filterGallery")
    } else if app.wide_angle.is_some() {
        Some(photocraft_engine::lens_cmds::WIDE)
    } else {
        None
    }
}

fn active_doc(app: &PhotocraftApp) -> Option<DocId> {
    app.session.active().map(|st| st.doc.id)
}

fn on_doc(app: &PhotocraftApp, document: DocId) -> bool {
    active_doc(app) == Some(document)
}

/// Drops Liquify / Select and Mask / Puppet / Perspective Warp / Vanishing Point / Filter Gallery when the
/// document they were opened on is no longer active, so they cannot steal undo, block layer
/// delete, or draw another document's pixels (#5115).
pub fn cancel_stale(app: &mut PhotocraftApp) {
    let active = active_doc(app);
    if app.distort.liquify.as_ref().is_some_and(|d| Some(d.document) != active) {
        crate::liquify_ui::cancel(app);
    }
    if app.distort.puppet.as_ref().is_some_and(|p| Some(p.document) != active) {
        app.distort.puppet = None;
    }
    if app.distort.perspective.as_ref().is_some_and(|p| Some(p.document) != active) {
        app.distort.perspective = None;
    }
    if app.distort.vanishing.as_ref().is_some_and(|v| Some(v.document) != active) {
        app.distort.vanishing = None;
    }
    if app.distort.gallery.as_ref().is_some_and(|g| Some(g.document) != active) {
        app.distort.gallery = None;
    }
    // Its document, or the layer it refines, is no longer the active one (#8896 review).
    if crate::select_mask_ui::stale(app).is_some() {
        crate::select_mask_ui::cancel(app);
    }
}

/// The document shown on the canvas while Puppet or Perspective Warp previews a layer, or
/// Vanishing Point's Stamp has painted.
pub fn display_doc(app: &PhotocraftApp, idx: usize) -> Option<(Arc<Document>, u64)> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    let (id, revision) = app.session.documents().get(idx).map(|st| (st.doc.id, st.revision))?;
    if let Some(p) = &app.distort.puppet
        && p.document == id
    {
        return Some((p.preview_doc.clone(), (1 << 42) + p.key));
    }
    if let Some(p) = &app.distort.perspective
        && p.document == id
    {
        return Some((p.preview_doc.clone(), (1 << 42) + p.key));
    }
    // Vanishing Point: the clone strokes painted so far, on the document as they were painted.
    app.distort.vanishing.as_ref().and_then(|v| v.shown_on(id, revision))
}

/// Menu / control routing. `Some` when this module handles the invocation.
pub fn menu(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    let empty = params.as_object().is_none_or(|o| o.is_empty());
    let ui = params.get("ui");
    match id {
        "filter.filterGallery" if empty => Some(crate::gallery_ui::open(app).map(|_| json!({"gallery": app.distort.describe()["gallery"]}))),
        "filter.filterGallery" if ui.is_some() && app.distort.gallery.is_some() => Some(crate::gallery_ui::control(app, ui.unwrap_or(&Value::Null))),
        "filter.liquify" if empty => Some(crate::liquify_ui::open(app, ctx).map(|_| json!({"liquify": app.distort.describe()["liquify"]}))),
        "filter.liquify" if ui.is_some() && app.distort.liquify.is_some() => Some(crate::liquify_ui::control(app, ui.unwrap_or(&Value::Null))),
        "edit.contentAwareFill" if ui.is_some() && app.distort.caf.is_some() => Some(crate::caf_ui::control(app, ui.unwrap_or(&Value::Null))),
        // `ui` addresses the workspace: closed, that is an error, not a fill with the defaults.
        "edit.contentAwareFill" if ui.is_some() => Some(Err("Content-Aware Fill is not open".into())),
        "select.selectAndMask" if ui.is_some() && app.distort.select_mask.is_some() => Some(crate::select_mask_ui::control(app, ui.unwrap_or(&Value::Null))),
        "select.selectAndMask" if ui.is_some() => Some(Err("Select and Mask is not open".into())),
        "select.selectAndMask" => Some(crate::select_mask_ui::open(app).map(|_| json!({"selectAndMask": app.distort.describe()["selectAndMask"]}))),
        "edit.puppetWarp" | "layer.smartObjects.puppetWarp" if empty => {
            Some(crate::puppet_ui::begin(app, ctx, id).map(|_| json!({"puppet": app.distort.describe()["puppet"]})))
        }
        "edit.puppetWarp" | "layer.smartObjects.puppetWarp" if ui.is_some() && app.distort.puppet.is_some() => {
            Some(crate::puppet_ui::control(app, ui.unwrap_or(&Value::Null)))
        }
        "edit.perspectiveWarp" | "layer.smartObjects.perspectiveWarp" if empty => {
            Some(crate::perspective_ui::begin(app, ctx, id).map(|_| json!({"perspective": app.distort.describe()["perspective"]})))
        }
        "edit.perspectiveWarp" | "layer.smartObjects.perspectiveWarp" if ui.is_some() && app.distort.perspective.is_some() => {
            Some(crate::perspective_ui::control(app, ui.unwrap_or(&Value::Null)))
        }
        "filter.vanishingPoint" if empty => Some(crate::vp_ui::begin(app, ctx).map(|_| json!({"vanishingPoint": app.distort.describe()["vanishingPoint"]}))),
        "filter.vanishingPoint" if ui.is_some() && app.distort.vanishing.is_some() => Some(crate::vp_ui::control(app, ui.unwrap_or(&Value::Null))),
        _ => None,
    }
}

/// Pointer events (document coordinates) while a mode is active. True when consumed.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    cancel_stale(app);
    if app.distort.caf.is_some() {
        crate::caf_ui::pointer(app, ev, mods);
        return true;
    }
    if app.distort.select_mask.as_ref().is_some_and(|w| on_doc(app, w.document)) {
        crate::select_mask_ui::pointer(app, ev, mods);
        return true;
    }
    if app.distort.liquify.as_ref().is_some_and(|d| on_doc(app, d.document)) {
        crate::liquify_ui::pointer(app, ev, mods);
        return true;
    }
    if app.distort.puppet.as_ref().is_some_and(|p| on_doc(app, p.document)) {
        crate::puppet_ui::pointer(app, ev, mods);
        return true;
    }
    if app.distort.perspective.as_ref().is_some_and(|p| on_doc(app, p.document)) {
        crate::perspective_ui::pointer(app, ev, mods);
        return true;
    }
    if app.distort.vanishing.as_ref().is_some_and(|v| on_doc(app, v.document)) {
        crate::vp_ui::pointer(app, ev, mods);
        return true;
    }
    false
}

/// Keyboard: ↩ commits, Esc cancels (and Liquify's own shortcuts). True when the key handling
/// of the rest of the app should be skipped.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use egui::{Key, Modifiers};
    cancel_stale(app);
    let enter = |ctx: &egui::Context| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter));
    let esc = |ctx: &egui::Context| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape));
    if app.distort.gallery.is_some() {
        if enter(ctx) {
            let _ = crate::gallery_ui::commit(app);
        } else if esc(ctx) {
            app.distort.gallery = None;
        }
        return true;
    }
    if app.distort.caf.is_some() {
        if enter(ctx) {
            crate::caf_ui::commit(app);
        } else if esc(ctx) {
            crate::caf_ui::cancel(app);
        }
        return true;
    }
    if app.distort.select_mask.is_some() {
        // Enter closes a Polygonal Lasso polygon, else OK; Esc drops one, else Cancel.
        if enter(ctx) {
            crate::select_mask_ui::enter(app);
        } else if esc(ctx) {
            crate::select_mask_ui::escape(app);
        }
        return true;
    }
    if app.distort.liquify.is_some() {
        if enter(ctx) {
            let _ = crate::liquify_ui::commit(app);
        } else if esc(ctx) {
            crate::liquify_ui::cancel(app);
        } else {
            crate::liquify_ui::keys(app, ctx);
        }
        return true;
    }
    if let Some(p) = app.distort.puppet.as_mut() {
        let cmd_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        if enter(ctx) {
            crate::puppet_ui::commit(app);
        } else if esc(ctx) {
            app.distort.puppet = None;
        } else if ctx.input_mut(|i| i.consume_key(cmd_shift, egui::Key::Z)) {
            p.redo();
        } else if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z)) {
            p.undo();
        } else if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace)) {
            p.delete_selected_pin();
        }
        return true;
    }
    if app.distort.perspective.is_some() {
        if enter(ctx) {
            crate::perspective_ui::commit(app);
        } else if esc(ctx) {
            app.distort.perspective = None;
        }
        return true;
    }
    if app.distort.vanishing.is_some() {
        if enter(ctx) {
            let _ = crate::vp_ui::commit(app);
        } else if esc(ctx) {
            crate::vp_ui::escape(app);
        } else {
            crate::vp_ui::keys(app, ctx);
        }
        return true;
    }
    false
}

/// The warped (or pasted) pixels of the open on-canvas mode: image content, which the canvas
/// draws right after the document and under the grid and guides (#7781), as Free Transform's
/// preview is (#5086). Its pins, planes and meshes are [`draw_overlay`], drawn over the guides.
pub fn draw_content(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if let Some(p) = &app.distort.puppet {
        crate::puppet_ui::draw_content(p, painter, xf);
    }
    if let Some(p) = &app.distort.perspective {
        crate::perspective_ui::draw_content(p, painter, xf);
    }
    if let Some(v) = &app.distort.vanishing {
        crate::vp_ui::draw_content(v, painter, xf);
    }
}

/// Canvas overlay for the on-canvas modes: pins, planes and meshes over the guides.
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    if let Some(p) = &app.distort.puppet
        && on_doc(app, p.document)
    {
        crate::puppet_ui::draw(p, painter, xf);
    }
    if let Some(p) = &app.distort.perspective
        && on_doc(app, p.document)
    {
        crate::perspective_ui::draw(p, painter, xf);
    }
    if let Some(v) = &app.distort.vanishing
        && on_doc(app, v.document)
    {
        crate::vp_ui::draw(v, painter, xf);
    }
}

/// Options bar for the on-canvas modes. True when drawn.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> bool {
    cancel_stale(app);
    if app.distort.puppet.is_some() {
        crate::puppet_ui::options_bar(app, ui);
        return true;
    }
    if app.distort.perspective.is_some() {
        crate::perspective_ui::options_bar(app, ui);
        return true;
    }
    if app.distort.vanishing.is_some() {
        crate::vp_ui::options_bar(app, ui);
        return true;
    }
    false
}

/// Drops the open sessions once the active document is not the one they were started on (closed, or another one
/// opened or switched to). Without this, Puppet Warp's preview of the old document replaced the new document's canvas (#5092).
pub fn drop_foreign(app: &mut PhotocraftApp) {
    let current = app.session.active().map(|s| s.doc.id);
    if !app.distort.active() && app.distort.gallery.is_none() {
        app.distort.owner = None;
    } else if app.distort.owner.is_none() {
        app.distort.owner = current;
    } else if app.distort.owner != current {
        app.distort.liquify = None;
        // Stops its background preview too; what Apply wrote stays, as with Cancel.
        crate::caf_ui::cancel(app);
        crate::select_mask_ui::cancel(app);
        app.distort.puppet = None;
        app.distort.perspective = None;
        app.distort.vanishing = None;
        app.distort.gallery = None;
        app.distort.owner = None;
    }
}

/// Full-window dialogs (Liquify, Content-Aware Fill, Filter Gallery).
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    cancel_stale(app);
    drop_foreign(app);
    if app.distort.caf.is_some() {
        crate::caf_ui::show(app, ctx);
    }
    if app.distort.select_mask.is_some() {
        crate::select_mask_ui::show(app, ctx);
    }
    if app.distort.gallery.is_some() {
        crate::gallery_ui::show(app, ctx);
    }
    if app.distort.liquify.is_some() {
        crate::liquify_ui::show(app, ctx);
    }
}

/// Screen distance (document px) that counts as "on" a handle at the current zoom.
pub(crate) fn tolerance(app: &PhotocraftApp) -> f64 {
    let z = app.point_zoom().max(0.01);
    8.0 / f64::from(z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Rect;
    use serde_json::json;

    fn paint_bar(app: &mut PhotocraftApp) {
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
    }

    /// #5115: Puppet Warp is per-document. Switching tabs drops the session so Ctrl+Z and
    /// layer delete apply to the new document, and the warp mesh does not leak onto it.
    #[test]
    fn switching_documents_drops_puppet_and_restores_undo_and_delete() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        paint_bar(&mut app);
        app.sync_views();
        menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        pointer(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x: 50.0, y: 40.0 }, egui::Modifiers::NONE);
        assert!(app.distort.puppet.is_some());
        let first = app.session.active().unwrap().doc.id;
        app.session.execute("file.new", json!({"width": 60, "height": 40, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.sync_views();
        assert!(app.distort.puppet.is_none(), "puppet does not follow the new document");
        assert!(display_doc(&app, app.session.active_index().unwrap()).is_none());
        let layers_before = app.session.active().unwrap().doc.layers.len();
        app.session.execute("layer.delete", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers_before - 1);
        assert!(app.session.set_active(0));
        app.sync_views();
        assert_eq!(app.session.active().unwrap().doc.id, first);
        let past = app.session.active().unwrap().history.past_len();
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), past - 1);
    }

    #[test]
    fn committed_puppet_is_one_undo_step_and_the_layer_stays_deletable() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        paint_bar(&mut app);
        app.sync_views();
        menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        pointer(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x: 25.0, y: 40.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Down { x: 75.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Move { x: 75.0, y: 30.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x: 75.0, y: 20.0 }, egui::Modifiers::NONE);
        let before = app.session.active().unwrap().history.past_len();
        crate::puppet_ui::commit(&mut app);
        assert!(app.distort.puppet.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), before);
        let layers = app.session.active().unwrap().doc.layers.len();
        app.session.execute("layer.delete", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().doc.layers.len(), layers - 1);
    }

    #[test]
    fn switching_documents_drops_liquify() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        paint_bar(&mut app);
        crate::liquify_ui::open(&mut app, &ctx).unwrap();
        assert!(app.distort.liquify.is_some());
        app.session.execute("file.new", json!({"width": 40, "height": 40, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.sync_views();
        assert!(app.distort.liquify.is_none());
        let past = app.session.active().unwrap().history.past_len();
        app.session.execute("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.active().unwrap().history.past_len(), past - 1);
    }
}
