//! Edit › Puppet Warp on the canvas: click on the mesh to add pins, drag pins to move them,
//! Delete/Backspace removes the selected pin; ⌥-click removes a pin under the pointer.
//! The options bar sets mode, density, expansion, mesh visibility,
//! the selected pin's depth and rotation. ⌘Z / ⌘⇧Z undo and redo pin edits inside the session
//! (#4440). ↩ / Commit runs `edit.puppetWarp` with the pins.
//!
//! The preview is the layer texture on the deformed triangle mesh (an egui textured mesh, drawn
//! by the GPU). Each changed pin position uses the engine's iteration count and rest-mesh
//! initialization, reusing the cached solver factorization. Warm-starting from earlier drag
//! previews made the result depend on pointer history and snap to a different shape on release.

use std::sync::Arc;

use egui::{Color32, Stroke, TextureHandle, pos2, vec2};
use photocraft_algo::puppet::{ITERATIONS, PuppetDensity, PuppetMesh, PuppetMode, PuppetPin, PuppetSolver, PuppetWarp, build_mesh};
use photocraft_doc::{Document, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};

pub struct PuppetSession {
    pub key: u64,
    pub layer: LayerId,
    /// Command to commit with (`edit.puppetWarp` or `layer.smartObjects.puppetWarp`).
    command: String,
    surface: Surface,
    bounds: Rect,
    pub warp: PuppetWarp,
    pub show_mesh: bool,
    pub selected: Option<usize>,
    drag: Option<usize>,
    mesh: PuppetMesh,
    solver: Option<PuppetSolver>,
    pub deformed: Vec<[f64; 2]>,
    order: Vec<usize>,
    texture: TextureHandle,
    opacity: f32,
    pub preview_doc: Arc<Document>,
    /// Pins before each edit of this session, newest last (⌘Z), and the edits undone (⌘⇧Z).
    undo: Vec<Vec<PuppetPin>>,
    redo: Vec<Vec<PuppetPin>>,
    /// The pins when the current drag started: one undo step per drag that moved something.
    drag_from: Option<Vec<PuppetPin>>,
}

impl PuppetSession {
    /// Records `before` as one undoable pin edit.
    fn record(&mut self, before: Vec<PuppetPin>) {
        if before != self.warp.pins {
            self.undo.push(before);
            self.redo.clear();
        }
    }

    /// Puts `pins` back (undo or redo), keeping the selection when that pin still exists.
    fn restore(&mut self, pins: Vec<PuppetPin>) {
        self.warp.pins = pins;
        self.selected = self.selected.filter(|i| *i < self.warp.pins.len());
        self.drag = None;
        self.drag_from = None;
        self.rebind();
    }

    /// ⌘Z: the last pin edit undone. False when there is none.
    pub fn undo(&mut self) -> bool {
        let Some(pins) = self.undo.pop() else { return false };
        self.redo.push(std::mem::replace(&mut self.warp.pins, pins.clone()));
        self.restore(pins);
        true
    }

    /// ⌘⇧Z: the last undone pin edit redone. False when there is none.
    pub fn redo(&mut self) -> bool {
        let Some(pins) = self.redo.pop() else { return false };
        self.undo.push(std::mem::replace(&mut self.warp.pins, pins.clone()));
        self.restore(pins);
        true
    }

    /// Removes the selected constraint and its deformation as one undoable pin edit.
    pub fn delete_selected_pin(&mut self) -> bool {
        let Some(i) = self.selected.filter(|i| *i < self.warp.pins.len()) else { return false };
        // Finish an in-progress drag first, so undo can restore the pin, then undo its move.
        if let Some(before) = self.drag_from.take() {
            self.record(before);
        }
        let before = self.warp.pins.clone();
        self.warp.pins.remove(i);
        self.selected = None;
        self.drag = None;
        self.rebind();
        self.record(before);
        true
    }

    pub fn describe(&self) -> Value {
        json!({
            "layer": self.layer.0,
            "pins": self.warp.pins,
            "mode": self.warp.mode,
            "density": self.warp.density,
            "expansion": self.warp.expansion,
            "vertices": self.mesh.verts.len(),
            "triangles": self.mesh.tris.len(),
            "selected": self.selected,
        })
    }

    fn rebuild_mesh(&mut self) {
        self.mesh = build_mesh(&self.surface, self.bounds, self.warp.density, self.warp.expansion);
        self.rebind();
    }

    /// Remeshes for a new density or expansion. One that leaves no pixels is refused, as `begin`
    /// refuses a layer without any: the previous `density` and `expansion` and their mesh come
    /// back (#951).
    fn remesh_or_restore(&mut self, density: PuppetDensity, expansion: f64) -> Result<(), String> {
        self.rebuild_mesh();
        if self.mesh.tris.is_empty() {
            self.warp.density = density;
            self.warp.expansion = expansion;
            self.rebuild_mesh();
            return Err("Could not use Puppet Warp: no opaque pixels at this density and expansion".into());
        }
        Ok(())
    }

    /// New solver for the current pin set (pins added/removed, rotation fixed or freed).
    fn rebind(&mut self) {
        let src: Vec<[f64; 2]> = self.warp.pins.iter().map(|p| p.src).collect();
        let fixed: Vec<bool> = self.warp.pins.iter().map(|p| p.rotate.is_some()).collect();
        self.solver = Some(PuppetSolver::new(self.mesh.clone(), &src, &fixed));
        self.solve();
    }

    /// Solves from rest, exactly like the engine, during dragging as well as at rest.
    fn solve(&mut self) {
        let Some(solver) = &self.solver else { return };
        let dst: Vec<[f64; 2]> = self.warp.pins.iter().map(|p| p.dst).collect();
        let rot: Vec<Option<f64>> = self.warp.pins.iter().map(|p| p.rotate).collect();
        let (v, _) = solver.solve(&dst, &rot, self.warp.mode, ITERATIONS, None);
        self.deformed = v;
        let src: Vec<[f64; 2]> = self.warp.pins.iter().map(|p| p.src).collect();
        let depths: Vec<i32> = self.warp.pins.iter().map(|p| p.depth).collect();
        self.order = solver.draw_order(&depths, &src);
    }

    fn pin_at(&self, p: [f64; 2], tol: f64) -> Option<usize> {
        self.warp
            .pins
            .iter()
            .enumerate()
            .filter(|(_, q)| (q.dst[0] - p[0]).hypot(q.dst[1] - p[1]) <= tol)
            .min_by(|a, b| dist(a.1.dst, p).total_cmp(&dist(b.1.dst, p)))
            .map(|(i, _)| i)
    }

    /// The rest position of a point on the deformed mesh (where a new pin attaches).
    fn rest_of(&self, p: [f64; 2]) -> Option<[f64; 2]> {
        for &t in self.order.iter().rev() {
            let tri = self.mesh.tris[t];
            let [a, b, c] = tri.map(|i| self.deformed[i]);
            let det = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
            if det.abs() < 1e-12 {
                continue;
            }
            let l1 = ((p[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (p[1] - a[1])) / det;
            let l2 = ((b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1])) / det;
            let l0 = 1.0 - l1 - l2;
            if l0 >= 0.0 && l1 >= 0.0 && l2 >= 0.0 {
                let [ra, rb, rc] = tri.map(|i| self.mesh.verts[i]);
                return Some([l0 * ra[0] + l1 * rb[0] + l2 * rc[0], l0 * ra[1] + l1 * rb[1] + l2 * rc[1]]);
            }
        }
        None
    }
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

/// Starts Puppet Warp on the active layer.
pub fn begin(app: &mut PhotocraftApp, ctx: &egui::Context, command: &str) -> Result<(), String> {
    if app.ui.transform.is_some() {
        return Err("finish Free Transform first".into());
    }
    let (layer, surface, _) = crate::distort_ui::active_pixels(app)?;
    let st = app.session.active().ok_or("no document")?;
    let l = st.doc.layer(layer).ok_or("no layer")?;
    let bounds = surface.content_bounds();
    if bounds.is_empty() {
        return Err("Could not use Puppet Warp: the layer is empty".into());
    }
    let opacity = l.opacity * l.fill_opacity;
    let preview_doc = crate::distort_ui::without_layer(&st.doc, layer);
    let img = crate::distort_ui::surface_image(&surface, bounds, 2048);
    let key = app.ui.alloc_id();
    let texture = ctx.load_texture(format!("puppet-{key}"), img, egui::TextureOptions::LINEAR);
    let mut s = PuppetSession {
        key,
        layer,
        command: command.to_string(),
        surface,
        bounds,
        warp: PuppetWarp { pins: Vec::new(), mode: PuppetMode::Normal, density: PuppetDensity::Normal, expansion: 2.0 },
        show_mesh: true,
        selected: None,
        drag: None,
        mesh: PuppetMesh::default(),
        solver: None,
        deformed: Vec::new(),
        order: Vec::new(),
        texture,
        opacity,
        preview_doc,
        undo: Vec::new(),
        redo: Vec::new(),
        drag_from: None,
    };
    s.rebuild_mesh();
    if s.mesh.tris.is_empty() {
        return Err("Could not use Puppet Warp: no opaque pixels".into());
    }
    app.distort.puppet = Some(s);
    Ok(())
}

pub fn commit(app: &mut PhotocraftApp) {
    let Some(s) = app.distort.puppet.take() else { return };
    if s.warp.is_identity() {
        return;
    }
    let mut p = serde_json::to_value(&s.warp).unwrap_or_default();
    p["layer"] = json!(s.layer.0);
    let _ = app.run(&s.command, p);
}

/// Control channel: `edit.puppetWarp {"ui": {...}}` while the mode is active.
pub fn control(app: &mut PhotocraftApp, ui: &Value) -> Result<Value, String> {
    if ui.get("commit").and_then(Value::as_bool) == Some(true) {
        commit(app);
        return Ok(json!({"committed": true}));
    }
    if ui.get("cancel").and_then(Value::as_bool) == Some(true) {
        app.distort.puppet = None;
        return Ok(json!({"cancelled": true}));
    }
    let s = app.distort.puppet.as_mut().ok_or(tl!("Puppet Warp is not active"))?;
    if ui.get("undo").and_then(Value::as_bool) == Some(true) {
        s.undo();
        return Ok(s.describe());
    }
    if ui.get("redo").and_then(Value::as_bool) == Some(true) {
        s.redo();
        return Ok(s.describe());
    }
    if ui.get("deleteSelectedPin").and_then(Value::as_bool) == Some(true) {
        s.delete_selected_pin();
        return Ok(s.describe());
    }
    let mut remesh = false;
    let (density, expansion) = (s.warp.density, s.warp.expansion);
    if let Some(m) = ui.get("mode").and_then(Value::as_str) {
        s.warp.mode = PuppetMode::parse(m).ok_or("mode: rigid|normal|distort")?;
    }
    if let Some(d) = ui.get("density").and_then(Value::as_str) {
        s.warp.density = PuppetDensity::parse(d).ok_or("density: fewer|normal|more")?;
        remesh = true;
    }
    if let Some(e) = ui.get("expansion").and_then(Value::as_f64) {
        s.warp.expansion = e.clamp(-200.0, 200.0);
        remesh = true;
    }
    if let Some(v) = ui.get("showMesh").and_then(Value::as_bool) {
        s.show_mesh = v;
    }
    if remesh {
        s.remesh_or_restore(density, expansion)?;
    } else {
        s.solve();
    }
    Ok(s.describe())
}

pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    let tol = crate::distort_ui::tolerance(app);
    let Some(s) = app.distort.puppet.as_mut() else { return };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            let p = [x, y];
            let before = s.warp.pins.clone();
            if let Some(i) = s.pin_at(p, tol) {
                if mods.alt {
                    s.selected = Some(i);
                    s.delete_selected_pin();
                } else {
                    s.selected = Some(i);
                    s.drag = Some(i);
                    s.drag_from = Some(before);
                }
            } else if !mods.alt
                && let Some(src) = s.rest_of(p)
            {
                s.warp.pins.push(PuppetPin { src, dst: p, rotate: None, depth: 0 });
                s.selected = Some(s.warp.pins.len() - 1);
                s.drag = s.selected;
                s.rebind();
                // Placing a pin is one step; dragging it on from there is part of that step.
                s.record(before);
            }
        }
        ToolEvent::Move { x, y, .. } => {
            if let Some(i) = s.drag
                && let Some(pin) = s.warp.pins.get_mut(i)
                && pin.dst != [x, y]
            {
                pin.dst = [x, y];
                s.solve();
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some(i) = s.drag.take() {
                // A release usually repeats the last Move, but can carry a new position.
                if let Some(pin) = s.warp.pins.get_mut(i)
                    && pin.dst != [x, y]
                {
                    pin.dst = [x, y];
                    s.solve();
                }
                if let Some(before) = s.drag_from.take() {
                    s.record(before);
                }
            }
        }
    }
}

/// The warped pixels: image content, drawn under the grid and guides like the document so a
/// guide stays visible across the layer being warped (#7781). The mesh lines and pins are [`draw`].
pub fn draw_content(s: &PuppetSession, painter: &egui::Painter, xf: &ViewXform) {
    let b = s.bounds;
    let (w, h) = (f64::from(b.width()), f64::from(b.height()));
    let mut mesh = egui::Mesh::with_texture(s.texture.id());
    let tint = Color32::from_white_alpha((s.opacity.clamp(0.0, 1.0) * 255.0) as u8);
    for (d, r) in s.deformed.iter().zip(&s.mesh.verts) {
        let uv = pos2(((r[0] - f64::from(b.x0)) / w) as f32, ((r[1] - f64::from(b.y0)) / h) as f32);
        mesh.vertices.push(egui::epaint::Vertex { pos: xf.to_screen(d[0] as f32, d[1] as f32), uv, color: tint });
    }
    for &t in &s.order {
        let [a, bb, c] = s.mesh.tris[t];
        mesh.add_triangle(a as u32, bb as u32, c as u32);
    }
    painter.add(mesh);
}

/// The mesh lines and pins over the warped pixels ([`draw_content`]) and the guides.
pub fn draw(s: &PuppetSession, painter: &egui::Painter, xf: &ViewXform) {
    if s.show_mesh {
        let line = Stroke::new(0.6, Color32::from_rgba_unmultiplied(40, 40, 40, 150));
        let mut seen = std::collections::HashSet::new();
        for tri in &s.mesh.tris {
            for k in 0..3 {
                let (i, j) = (tri[k].min(tri[(k + 1) % 3]), tri[k].max(tri[(k + 1) % 3]));
                if seen.insert((i, j)) {
                    let (p, q) = (s.deformed[i], s.deformed[j]);
                    painter.line_segment([xf.to_screen(p[0] as f32, p[1] as f32), xf.to_screen(q[0] as f32, q[1] as f32)], line);
                }
            }
        }
    }
    // Pins: yellow discs with a black ring; the selected pin has a black centre (Photoshop).
    for (i, p) in s.warp.pins.iter().enumerate() {
        let c = xf.to_screen(p.dst[0] as f32, p.dst[1] as f32);
        painter.circle_filled(c, 5.5, Color32::from_rgb(255, 214, 10));
        painter.circle_stroke(c, 5.5, Stroke::new(1.25, Color32::BLACK));
        if s.selected == Some(i) {
            painter.circle_filled(c, 2.0, Color32::BLACK);
            if let Some(r) = p.rotate {
                let a = (r as f32).to_radians();
                painter.circle_stroke(c, 18.0, Stroke::new(1.0, Color32::WHITE));
                painter.line_segment([c, c + vec2(a.cos(), a.sin()) * 18.0], Stroke::new(1.0, Color32::WHITE));
            }
        }
    }
}

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(s) = app.distort.puppet.as_mut() else { return };
    // Pin depth, rotation and Remove All Pins are undoable pin edits too.
    let pins_before = s.warp.pins.clone();
    let mut remesh = false;
    let mut resolve = false;
    let mut rebind = false;
    ui.label(tl!("Mode:"));
    let mode = s.warp.mode;
    crate::widgets::dropdown(
        ui,
        "puppet-mode",
        &mut s.warp.mode,
        &[(PuppetMode::Rigid, tl!("Rigid")), (PuppetMode::Normal, tl!("Normal")), (PuppetMode::Distort, tl!("Distort"))],
        90.0,
    );
    resolve |= mode != s.warp.mode;
    ui.label(tl!("Density:"));
    let density = s.warp.density;
    crate::widgets::dropdown(
        ui,
        "puppet-density",
        &mut s.warp.density,
        &[(PuppetDensity::Fewer, tl!("Fewer Points")), (PuppetDensity::Normal, tl!("Normal")), (PuppetDensity::More, tl!("More Points"))],
        110.0,
    );
    remesh |= density != s.warp.density;
    ui.label(tl!("Expansion:"));
    let expansion = s.warp.expansion;
    let mut e = s.warp.expansion as f32;
    if crate::widgets::value_field(ui, &mut e, -50.0..=100.0, "px", 56.0).changed() {
        s.warp.expansion = f64::from(e);
        remesh = true;
    }
    crate::widgets::checkbox(ui, &mut s.show_mesh, tl!("Show Mesh"));
    crate::widgets::vline(ui, 22.0);
    if let Some(i) = s.selected.filter(|i| *i < s.warp.pins.len()) {
        ui.label(tl!("Pin Depth:"));
        if ui.small_button("+").on_hover_text(tl!("Bring the selected pin forward")).clicked() {
            s.warp.pins[i].depth += 1;
            resolve = true;
        }
        if ui.small_button("−").on_hover_text(tl!("Send the selected pin backward")).clicked() {
            s.warp.pins[i].depth -= 1;
            resolve = true;
        }
        ui.label(tl!("Rotate:"));
        let mut fixed = s.warp.pins[i].rotate.is_some();
        let was = fixed;
        crate::widgets::dropdown(ui, "puppet-rotate", &mut fixed, &[(false, tl!("Auto")), (true, tl!("Fixed"))], 70.0);
        if fixed != was {
            s.warp.pins[i].rotate = fixed.then_some(0.0);
            rebind = true;
        }
        if let Some(r) = s.warp.pins[i].rotate {
            let mut a = r as f32;
            if crate::widgets::value_field(ui, &mut a, -360.0..=360.0, "°", 52.0).changed() {
                s.warp.pins[i].rotate = Some(f64::from(a));
                resolve = true;
            }
        }
        crate::widgets::vline(ui, 22.0);
    }
    if ui.button(tl!("Remove All Pins")).clicked() {
        s.warp.pins.clear();
        s.selected = None;
        rebind = true;
    }
    s.record(pins_before);
    if remesh {
        // A refused value snaps the fields back to the previous settings.
        let _ = s.remesh_or_restore(density, expansion);
    } else if rebind {
        s.rebind();
    } else if resolve {
        s.solve();
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if crate::widgets::primary_button(ui, "✓", 32.0)
            .on_hover_text(crate::i18n::fmt(tl!("Commit Puppet Warp ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]))
            .clicked()
        {
            commit(app);
        } else if crate::widgets::secondary_button(ui, "⊘", 32.0).on_hover_text(tl!("Cancel (Esc)")).clicked() {
            app.distort.puppet = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn puppet_app(ctx: &egui::Context) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        begin(&mut app, ctx, "edit.puppetWarp").unwrap();
        app
    }

    fn gesture(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
        pointer(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(app, ToolEvent::Up { x: to[0], y: to[1] }, egui::Modifiers::NONE);
    }

    fn press(app: &mut PhotocraftApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        let events = [true, false].map(|pressed| egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }).to_vec();
        let input = egui::RawInput { events, ..Default::default() };
        ctx.run_ui(input, |ui| crate::shortcuts::handle(app, ui.ctx())).textures_delta.clear();
    }

    #[test]
    fn delete_keys_remove_selected_pin_and_restore_its_deformation_on_undo() {
        for key in [egui::Key::Delete, egui::Key::Backspace] {
            let ctx = egui::Context::default();
            let mut app = puppet_app(&ctx);
            gesture(&mut app, [25.0, 40.0], [25.0, 40.0]);
            gesture(&mut app, [75.0, 40.0], [75.0, 40.0]);
            gesture(&mut app, [75.0, 40.0], [75.0, 30.0]);
            gesture(&mut app, [25.0, 40.0], [25.0, 20.0]);
            let s = app.distort.puppet.as_mut().unwrap();
            // Rotation/depth are part of a deleted pin's state, not just its position.
            let before = s.warp.pins.clone();
            s.warp.pins[0].rotate = Some(15.0);
            s.warp.pins[0].depth = 2;
            s.rebind();
            s.record(before);
            let pins = s.warp.pins.clone();
            let deformed = s.deformed.clone();
            let order = s.order.clone();
            let edits = s.undo.len();
            let survivor = &pins[1];
            let solver = PuppetSolver::new(s.mesh.clone(), &[survivor.src], &[survivor.rotate.is_some()]);
            let expected = solver.solve(&[survivor.dst], &[survivor.rotate], s.warp.mode, ITERATIONS, None).0;
            let past = app.session.active().unwrap().history.past_len();

            press(&mut app, &ctx, key, egui::Modifiers::NONE);
            let s = app.distort.puppet.as_ref().unwrap();
            assert_eq!(s.warp.pins, pins[1..], "{key:?}: delete the selected first pin, not the last");
            assert_eq!(s.selected, None);
            assert_eq!(s.deformed, expected, "solve from rest without the deleted constraint");
            assert_eq!(s.order, solver.draw_order(&[survivor.depth], &[survivor.src]));
            assert_eq!(s.undo.len(), edits + 1);
            press(&mut app, &ctx, key, egui::Modifiers::NONE);
            assert_eq!(app.distort.puppet.as_ref().unwrap().undo.len(), edits + 1, "no selection: no edit");

            press(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
            let s = app.distort.puppet.as_ref().unwrap();
            assert_eq!(s.warp.pins, pins);
            assert_eq!(s.deformed, deformed);
            assert_eq!(s.order, order);
            press(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
            assert_eq!(app.distort.puppet.as_ref().unwrap().deformed, expected);
            assert_eq!(app.session.active().unwrap().history.past_len(), past, "pin edits stay in the open session");

            let original = app.session.active().unwrap().doc.clone();
            commit(&mut app);
            assert_eq!(app.session.active().unwrap().history.past_len(), past + 1);
            let committed = app.session.active().unwrap().doc.clone();
            assert_ne!(committed, original);
            app.session.execute("edit.undo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, original);
            app.session.execute("edit.redo", json!({})).unwrap();
            assert_eq!(app.session.active().unwrap().doc, committed);
        }
    }

    #[test]
    fn deleting_during_a_drag_settles_history_and_cannot_move_the_next_pin() {
        let ctx = egui::Context::default();
        let mut app = puppet_app(&ctx);
        gesture(&mut app, [25.0, 40.0], [25.0, 40.0]);
        gesture(&mut app, [75.0, 40.0], [75.0, 40.0]);
        let original = app.distort.puppet.as_ref().unwrap().warp.pins.clone();
        pointer(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Move { x: 25.0, y: 20.0, pressure: 1.0 }, egui::Modifiers::NONE);
        let moved = app.distort.puppet.as_ref().unwrap().warp.pins.clone();
        press(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Move { x: 5.0, y: 5.0, pressure: 1.0 }, egui::Modifiers::NONE);
        pointer(&mut app, ToolEvent::Up { x: 5.0, y: 5.0 }, egui::Modifiers::NONE);
        let s = app.distort.puppet.as_mut().unwrap();
        assert_eq!(s.warp.pins, original[1..]);
        assert!(s.drag.is_none() && s.drag_from.is_none());
        assert!(s.undo());
        assert_eq!(s.warp.pins, moved, "first undo restores the deleted pin at its moved position");
        assert!(s.undo());
        assert_eq!(s.warp.pins, original, "second undo reverts the preceding drag");
        assert!(s.redo());
        s.selected = Some(0);
        control(&mut app, &json!({"deleteSelectedPin": true})).unwrap();
        assert!(!app.distort.puppet.as_mut().unwrap().redo(), "a new deletion discards the redo branch");
    }

    #[test]
    fn deleting_last_pin_restores_rest_mesh_and_empty_commit_is_not_an_edit() {
        let ctx = egui::Context::default();
        let mut app = puppet_app(&ctx);
        gesture(&mut app, [25.0, 40.0], [25.0, 20.0]);
        let past = app.session.active().unwrap().history.past_len();
        let s = app.distort.puppet.as_mut().unwrap();
        let pins = s.warp.pins.clone();
        s.selected = Some(usize::MAX);
        press(&mut app, &ctx, egui::Key::Delete, egui::Modifiers::NONE);
        assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins, pins, "stale selection is harmless");
        app.distort.puppet.as_mut().unwrap().selected = Some(0);
        press(&mut app, &ctx, egui::Key::Backspace, egui::Modifiers::NONE);
        let s = app.distort.puppet.as_ref().unwrap();
        assert!(s.warp.pins.is_empty());
        assert_eq!(s.deformed.len(), s.mesh.verts.len());
        for (deformed, rest) in s.deformed.iter().zip(&s.mesh.verts) {
            assert!(dist(*deformed, *rest) < 1e-8, "no constraints: the mesh returns to rest");
        }
        commit(&mut app);
        assert!(app.distort.puppet.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), past);
    }

    #[test]
    fn delete_keys_in_a_text_field_do_not_remove_pins() {
        for key in [egui::Key::Delete, egui::Key::Backspace] {
            let ctx = egui::Context::default();
            let mut app = puppet_app(&ctx);
            gesture(&mut app, [25.0, 40.0], [25.0, 20.0]);
            let mut text = "123".to_string();
            ctx.run_ui(Default::default(), |ui| {
                ui.text_edit_singleline(&mut text).request_focus();
            })
            .textures_delta
            .clear();
            let pins = app.distort.puppet.as_ref().unwrap().warp.pins.clone();
            let input = egui::RawInput {
                events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
                ..Default::default()
            };
            ctx.run_ui(input, |ui| {
                assert!(ui.ctx().text_edit_focused());
                crate::shortcuts::handle(&mut app, ui.ctx());
                ui.text_edit_singleline(&mut text);
            })
            .textures_delta
            .clear();
            assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins, pins);
            assert_eq!(app.distort.puppet.as_ref().unwrap().undo.len(), 1);
        }
    }

    fn long_puppet(ctx: &egui::Context, mode: PuppetMode, density: PuppetDensity) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 1000, "height": 300, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(40, 180, 960, 220), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        begin(&mut app, ctx, "edit.puppetWarp").unwrap();
        for x in [70.0, 160.0, 930.0] {
            pointer(&mut app, ToolEvent::Down { x, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
            pointer(&mut app, ToolEvent::Up { x, y: 200.0 }, egui::Modifiers::NONE);
        }
        let s = app.distort.puppet.as_mut().unwrap();
        s.warp.mode = mode;
        s.warp.density = density;
        s.rebuild_mesh();
        app
    }

    fn assert_same_mesh(a: &[[f64; 2]], b: &[[f64; 2]]) {
        assert_eq!(a.len(), b.len());
        let offset = a.iter().zip(b).map(|(a, b)| dist(*a, *b)).fold(0.0, f64::max);
        assert!(offset < 1e-8, "mesh moved by {offset} px with unchanged pins");
    }

    #[test]
    fn drag_preview_matches_release_history_and_committed_geometry() {
        use photocraft_algo::puppet::{deform, render};
        for mode in [PuppetMode::Rigid, PuppetMode::Normal, PuppetMode::Distort] {
            for density in [PuppetDensity::Fewer, PuppetDensity::Normal, PuppetDensity::More] {
                let ctx = egui::Context::default();
                let mut app = long_puppet(&ctx, mode, density);
                let rest = app.distort.puppet.as_ref().unwrap().deformed.clone();
                let past = app.session.active().unwrap().history.past_len();
                pointer(&mut app, ToolEvent::Down { x: 930.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
                for step in 1..=12 {
                    let x = 930.0 - f64::from(step) * 20.0;
                    let y = 200.0 - f64::from(step) * 10.0;
                    pointer(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
                }
                let s = app.distort.puppet.as_ref().unwrap();
                let preview = s.deformed.clone();
                let order = s.order.clone();
                pointer(&mut app, ToolEvent::Up { x: 690.0, y: 80.0 }, egui::Modifiers::NONE);
                let s = app.distort.puppet.as_mut().unwrap();
                assert_same_mesh(&preview, &s.deformed);
                assert_eq!(order, s.order);
                let (_, canonical, _) = deform(&s.surface, s.bounds, &s.warp, ITERATIONS);
                assert_same_mesh(&preview, &canonical);
                assert!(s.undo());
                assert_same_mesh(&rest, &s.deformed);
                assert!(s.redo());
                assert_same_mesh(&preview, &s.deformed);
                // Use the engine's interpolation to isolate geometry from GPU texture sampling.
                let expected = render(&s.surface, s.bounds, &s.mesh, &preview, &order, photocraft_algo::transform::Interp::Bicubic);
                let layer = s.layer;
                assert_eq!(app.session.active().unwrap().history.past_len(), past);
                commit(&mut app);
                let st = app.session.active().unwrap();
                assert_eq!(st.history.past_len(), past + 1);
                assert_eq!(st.doc.layer(layer).unwrap().surface().unwrap(), &expected);
            }
        }
    }

    #[test]
    fn drag_preview_is_independent_of_pointer_path_and_honors_release_coordinates() {
        let ctx = egui::Context::default();
        let mut app = long_puppet(&ctx, PuppetMode::Normal, PuppetDensity::More);
        let s = app.distort.puppet.as_mut().unwrap();
        s.warp.pins[0].rotate = Some(25.0);
        s.rebind();
        let pins = s.warp.pins.clone();
        let paths = [
            vec![[690.0, 80.0]],
            vec![[870.0, 170.0], [810.0, 140.0], [750.0, 110.0], [690.0, 80.0]],
            vec![[500.0, 30.0], [930.0, 200.0], [1000.0, 270.0], [690.0, 80.0]],
        ];
        let mut expected = None;
        for path in paths {
            app.distort.puppet.as_mut().unwrap().restore(pins.clone());
            pointer(&mut app, ToolEvent::Down { x: 930.0, y: 200.0, pressure: 1.0 }, egui::Modifiers::NONE);
            for [x, y] in path {
                pointer(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, egui::Modifiers::NONE);
            }
            let preview = &app.distort.puppet.as_ref().unwrap().deformed;
            assert_same_mesh(expected.get_or_insert_with(|| preview.clone()), preview);
            // The last pointer-up may carry a new position without a preceding move event.
            pointer(&mut app, ToolEvent::Up { x: 720.0, y: 90.0 }, egui::Modifiers::NONE);
            let s = app.distort.puppet.as_ref().unwrap();
            assert_eq!(s.warp.pins[2].dst, [720.0, 90.0]);
            let (_, canonical, _) = photocraft_algo::puppet::deform(&s.surface, s.bounds, &s.warp, ITERATIONS);
            assert_same_mesh(&s.deformed, &canonical);
        }
    }

    /// #5092: a Puppet Warp session left open does not follow the user to a new document.
    #[test]
    fn session_is_dropped_when_another_document_becomes_active() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        crate::distort_ui::drop_foreign(&mut app);
        assert!(app.distort.puppet.is_some());
        app.session.execute("file.new", json!({"width": 32, "height": 32, "depth": 8})).unwrap();
        assert!(crate::distort_ui::display_doc(&app, 1).is_some());
        crate::distort_ui::drop_foreign(&mut app);
        assert!(app.distort.puppet.is_none());
        assert!(crate::distort_ui::display_doc(&app, 1).is_none());
        // Closing the document it was started on drops it too.
        assert!(app.session.set_active(0));
        crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        crate::distort_ui::drop_foreign(&mut app);
        assert!(app.distort.puppet.is_some());
        assert!(app.session.close(0).is_some());
        crate::distort_ui::drop_foreign(&mut app);
        assert!(app.distort.puppet.is_none());
    }

    /// #4440: ⌘Z / ⌘⇧Z undo and redo pin edits while Puppet Warp is open: placing a pin,
    /// dragging one and removing one are a step each; a click that moves nothing isn't one.
    #[test]
    fn pin_edits_undo_and_redo_inside_the_session() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        let ev = |app: &mut PhotocraftApp, e| crate::distort_ui::pointer(app, e, egui::Modifiers::NONE);
        let pins = |app: &PhotocraftApp| app.distort.puppet.as_ref().unwrap().warp.pins.iter().map(|p| p.dst).collect::<Vec<_>>();
        ev(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 25.0, y: 40.0 });
        ev(&mut app, ToolEvent::Down { x: 75.0, y: 40.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 75.0, y: 40.0 });
        // Drag the second pin up; then a click on it that moves nothing.
        ev(&mut app, ToolEvent::Down { x: 75.0, y: 40.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Move { x: 75.0, y: 30.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 75.0, y: 20.0 });
        ev(&mut app, ToolEvent::Down { x: 75.0, y: 20.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 75.0, y: 20.0 });
        crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 }, egui::Modifiers { alt: true, ..Default::default() });
        assert_eq!(pins(&app), [[75.0, 20.0]]);
        // ⌘Z, through the keyboard: the removed pin comes back, then the drag is undone.
        let press = |app: &mut PhotocraftApp, modifiers: egui::Modifiers| {
            let ev = egui::Event::Key { key: egui::Key::Z, physical_key: None, pressed: true, repeat: false, modifiers };
            let input = egui::RawInput { events: vec![ev], ..Default::default() };
            ctx.run_ui(input, |ui| assert!(crate::distort_ui::keys(app, ui.ctx()))).textures_delta.clear();
        };
        press(&mut app, egui::Modifiers::COMMAND);
        assert_eq!(pins(&app), [[25.0, 40.0], [75.0, 20.0]]);
        press(&mut app, egui::Modifiers::COMMAND);
        assert_eq!(pins(&app), [[25.0, 40.0], [75.0, 40.0]], "the drag, not the click after it");
        // ⌘⇧Z redoes it; a new edit after an undo drops what was undone.
        press(&mut app, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        assert_eq!(pins(&app), [[25.0, 40.0], [75.0, 20.0]]);
        control(&mut app, &json!({"undo": true})).unwrap();
        control(&mut app, &json!({"undo": true})).unwrap();
        assert_eq!(pins(&app), [[25.0, 40.0]]);
        ev(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 50.0, y: 40.0 });
        control(&mut app, &json!({"redo": true})).unwrap();
        assert_eq!(pins(&app), [[25.0, 40.0], [50.0, 40.0]], "nothing left to redo");
        // Undo past the first pin stops there, and the document's History is untouched.
        let past = app.session.active().unwrap().history.past_len();
        for _ in 0..5 {
            control(&mut app, &json!({"undo": true})).unwrap();
        }
        assert!(pins(&app).is_empty());
        assert_eq!(app.session.active().unwrap().history.past_len(), past);
    }

    #[test]
    fn pins_drag_and_commit_through_the_engine() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        let ev = |app: &mut PhotocraftApp, e| crate::distort_ui::pointer(app, e, egui::Modifiers::NONE);
        // Two pins: hold the left end, drag the right end up.
        ev(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 25.0, y: 40.0 });
        ev(&mut app, ToolEvent::Down { x: 75.0, y: 40.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Move { x: 75.0, y: 30.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 75.0, y: 20.0 });
        let s = app.distort.puppet.as_ref().unwrap();
        assert_eq!(s.warp.pins.len(), 2);
        assert_eq!(s.warp.pins[1].dst, [75.0, 20.0]);
        // ⌥-click removes; clicking off the mesh adds nothing.
        ev(&mut app, ToolEvent::Down { x: 5.0, y: 5.0, pressure: 1.0 });
        assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins.len(), 2);
        crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 }, egui::Modifiers { alt: true, ..Default::default() });
        assert_eq!(app.distort.puppet.as_ref().unwrap().warp.pins.len(), 1);
        ev(&mut app, ToolEvent::Down { x: 25.0, y: 40.0, pressure: 1.0 });
        ev(&mut app, ToolEvent::Up { x: 25.0, y: 40.0 });
        control(&mut app, &json!({"mode": "rigid", "density": "more"})).unwrap();
        let expected = app.distort.puppet.as_ref().unwrap().deformed.clone();
        assert!(!expected.is_empty());
        let before = app.session.active().unwrap().history.past_len();
        crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({"ui": {"commit": true}})).unwrap().unwrap();
        assert!(app.distort.puppet.is_none());
        assert_eq!(app.session.active().unwrap().history.past_len(), before + 1);
        let st = app.session.active().unwrap();
        let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
        assert!(b.y0 < 25, "the right end went up: {b:?}");
    }

    #[test]
    fn an_expansion_that_empties_the_mesh_is_refused() {
        // #951: remeshing to no vertices rebound the pins to a vertex that doesn't exist and
        // panicked. -50 is the options bar's minimum and also empties this 20 px tall bar.
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.execute("file.new", json!({"width": 100, "height": 80, "depth": 8})).unwrap();
        app.session.execute("layer.new.layer", json!({})).unwrap();
        app.session
            .edit("paint", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(20, 30, 80, 50), &[1.0, 0.0, 0.0, 1.0]);
                Ok(())
            })
            .unwrap();
        app.sync_views();
        crate::distort_ui::menu(&mut app, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
        crate::distort_ui::pointer(&mut app, ToolEvent::Down { x: 50.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::distort_ui::pointer(&mut app, ToolEvent::Up { x: 50.0, y: 40.0 }, egui::Modifiers::NONE);
        let before = app.distort.puppet.as_ref().unwrap().describe();
        for e in [-200, -50] {
            assert!(control(&mut app, &json!({"expansion": e})).is_err(), "{e}");
            assert_eq!(app.distort.puppet.as_ref().unwrap().describe(), before, "{e}: the previous settings and mesh stay");
        }
        // A smaller shrink still remeshes.
        let v = control(&mut app, &json!({"expansion": -2})).unwrap();
        assert_eq!(v["expansion"], -2.0);
        assert!(v["triangles"].as_u64().unwrap() > 0);
    }
}
