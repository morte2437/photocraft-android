use super::editing::edit_home;
use super::ime_edit::{ime_offsets, ime_preedit_is_current};
use super::layout::layout;
use super::{Color32, LayerId, PhotocraftApp, Point, Pos2, Stroke, ViewXform, byte_of, text_layer};

/// Selection highlight, caret and text frame over the canvas.
pub fn draw_overlay(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    app.ui.type_box_overflow = false;
    let Some(ed) = app.ui.text_edit.clone() else { return };
    // Until it commits, an edit in a document no longer active draws nothing over this one.
    if edit_home(app, &ed) != app.session.active_index() {
        return;
    }
    if ed.dragging {
        app.ui.caret_blink_time = painter.ctx().input(|i| i.time);
    }
    let id = LayerId(ed.layer);
    let Some((l, aff, text)) = layout(app, id) else { return };
    if ed.mask {
        draw_mask_overlay(app, painter, xf, id);
    }
    let t = crate::theme::Tokens::get(painter.ctx());
    // Text space → screen.
    let scr_t = |x: f32, y: f32| -> Pos2 {
        let p = aff.apply(Point::new(x as f64, y as f64));
        xf.to_screen(p.x as f32, p.y as f32)
    };
    let shape = app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()));
    // Type on a path: the caret, selection and underline follow the curve the glyphs sit on.
    let text_start = l.bounds().map_or(0.0, |bounds| f64::from(bounds[0]));
    let bend = shape.as_ref().and_then(|shape| photocraft_text::warp::Warp::on_path_with_text_start(shape, text_start));
    // Line space (lines, clusters, carets) → screen: vertical type turns it 90° clockwise.
    let scr = |x: f32, y: f32| -> Pos2 {
        let (x, y) = l.to_text(x, y);
        match &bend {
            Some(w) => {
                let (x, y) = w.apply(f64::from(x), f64::from(y));
                scr_t(x as f32, y as f32)
            }
            None => scr_t(x, y),
        }
    };
    // A line-space run from `x0` to `x1` at height `y`: sampled, so it bends with path type.
    let run = |x0: f32, x1: f32, y: f32| -> Vec<Pos2> {
        let n = if bend.is_some() { ((x1 - x0) / 4.0).ceil().clamp(1.0, 512.0) as usize } else { 1 };
        (0..=n).map(|i| scr(x0 + (x1 - x0) * i as f32 / n as f32, y)).collect()
    };
    // The IME descriptor is emitted once at pass end by `sync_ime`, from this pass's canvas
    // mapping and the model as it stands after every panel has run.
    let pass_nr = painter.ctx().cumulative_pass_nr_for(painter.ctx().viewport_id());
    app.type_ime.note_view(painter.ctx().viewport_id(), pass_nr, xf);
    // Frame: paragraph text shows its box with handles; point text an underline per line.
    let mods = crate::workspace_ui::sticky_mods(app, painter.ctx().input(|i| i.modifiers));
    let has_transform = crate::type_transform::draw(app, painter, xf, mods);
    let frame = Stroke::new(1.0, t.accent);
    match shape {
        Some(photocraft_doc::text::TextShape::Box { x, y, width, height }) => {
            // Text the box can't hold: as in the reference application, the lower-right handle
            // grows and shows a black "+". Automation reads `ui.type_box_overflow` instead.
            let over = l.overflows(&text);
            app.ui.type_box_overflow = over;
            if !has_transform {
                let c = [scr_t(x, y), scr_t(x + width, y), scr_t(x + width, y + height), scr_t(x, y + height)];
                painter.add(egui::Shape::closed_line(c.to_vec(), frame));
                let mids = [c[0].lerp(c[1], 0.5), c[1].lerp(c[2], 0.5), c[2].lerp(c[3], 0.5), c[3].lerp(c[0], 0.5)];
                for (i, p) in c.iter().chain(mids.iter()).enumerate() {
                    let side = if over && i == 2 { OVERFLOW_HANDLE } else { 7.0 };
                    let r = egui::Rect::from_center_size(*p, egui::vec2(side, side));
                    painter.rect_filled(r, 0.0, Color32::WHITE);
                    painter.rect_stroke(r, 0.0, frame, egui::StrokeKind::Inside);
                }
                if over {
                    let (p, arm) = (c[2], OVERFLOW_HANDLE / 2.0 - 2.5);
                    painter.line_segment([p - egui::vec2(arm, 0.0), p + egui::vec2(arm, 0.0)], OVERFLOW_PLUS);
                    painter.line_segment([p - egui::vec2(0.0, arm), p + egui::vec2(0.0, arm)], OVERFLOW_PLUS);
                }
            }
        }
        // Area type: the outline of the shape it wraps inside.
        Some(photocraft_doc::text::TextShape::Area { path }) => {
            if !has_transform {
                for poly in photocraft_vector::flatten::flatten_path(&path, 0.5) {
                    let pts: Vec<Pos2> = poly.pts.iter().map(|&(x, y)| scr_t(x as f32, y as f32)).collect();
                    if pts.len() >= 2 {
                        painter.add(egui::Shape::closed_line(pts, frame));
                    }
                }
            }
        }
        _ => {
            for ln in &l.lines {
                // Horizontal: just under the baseline; vertical: the column's centre line.
                let y = if l.vertical { ln.baseline } else { ln.baseline + ln.descent * 0.25 };
                painter.add(egui::Shape::line(run(ln.x0.min(0.0), ln.x1.max(ln.x0 + 1.0), y), Stroke::new(1.0, t.accent.gamma_multiply(0.8))));
            }
        }
    }
    // Uncommitted IME text is underlined, but only while the stored range still holds exactly
    // the composition: an application edit may have shifted it onto committed characters.
    if let Some(offsets) = ime_offsets(&text, &ed)
        && ime_preedit_is_current(app, &text, &ed, &offsets)
        && let Some((a, b)) = offsets.preedit.map(|range| (range.start, range.end))
    {
        for (li, ln) in l.lines.iter().enumerate() {
            let xs: Vec<(f32, f32)> =
                l.clusters.iter().filter(|c| c.line == li && c.range.start >= a && c.range.end <= b).map(|c| (c.x, c.x + c.advance)).collect();
            let (x0, x1) = xs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (p, q)| (lo.min(*p), hi.max(*q)));
            if x0 < x1 {
                let y = ln.baseline + ln.descent * 0.5;
                painter.add(egui::Shape::line(run(x0, x1, y), Stroke::new(1.5, t.accent)));
            }
        }
    }
    // Selection: per line, the clusters inside [a, b).
    let (a, b) = (byte_of(&text, ed.caret.min(ed.anchor)), byte_of(&text, ed.caret.max(ed.anchor)));
    if a < b {
        let fill = Color32::from_rgba_unmultiplied(t.accent.r(), t.accent.g(), t.accent.b(), 110);
        for (li, ln) in l.lines.iter().enumerate() {
            let xs: Vec<(f32, f32)> =
                l.clusters.iter().filter(|c| c.line == li && c.range.start >= a && c.range.end <= b).map(|c| (c.x, c.x + c.advance)).collect();
            let (mut x0, mut x1) = xs.iter().fold((f32::MAX, f32::MIN), |(lo, hi), (p, q)| (lo.min(*p), hi.max(*q)));
            // A selected line break shows as a small sliver past the line end.
            if b > ln.range.end && a <= ln.range.end {
                x1 = x1.max(ln.x1 + (ln.ascent + ln.descent) * 0.25);
                x0 = x0.min(ln.x1);
            }
            let (top, bot) = (ln.baseline - ln.ascent, ln.baseline + ln.descent);
            if bend.is_some() {
                // On a path each glyph turns on its own, so each gets its own highlight.
                // Glyphs past the end of the path are not drawn, so neither is their highlight.
                let shown = |p: f32, q: f32| bend.as_ref().is_some_and(|w| w.place_glyph(f64::from(p + q) / 2.0) != photocraft_text::warp::Placement::Hidden);
                for (p, q) in xs.into_iter().filter(|&(p, q)| shown(p, q)) {
                    painter.add(egui::Shape::convex_polygon(vec![scr(p, top), scr(q, top), scr(q, bot), scr(p, bot)], fill, Stroke::NONE));
                }
            } else if x0 < x1 {
                painter.add(egui::Shape::convex_polygon(vec![scr(x0, top), scr(x1, top), scr(x1, bot), scr(x0, bot)], fill, Stroke::NONE));
            }
        }
    } else {
        // Blinking caret (Photoshop's ~0.53 s rhythm), solid while dragging or immediately after interaction.
        let time = painter.ctx().input(|i| i.time);
        let elapsed = (time - app.ui.caret_blink_time).max(0.0);
        if ed.dragging || (elapsed * 1000.0 / 530.0) as i64 % 2 == 0 {
            let (x, top, bot) = l.caret(a);
            let (x, top, bot) = if l.lines.is_empty() { (0.0, -(12.0 * l.px_per_pt.max(1.0)), 3.0) } else { (x, top, bot) };
            for stroke in CARET_STROKES {
                painter.line_segment([scr(x, top), scr(x, bot)], stroke);
            }
        }
    }
}

/// The side of a paragraph box's lower-right handle while the box can't hold its text (the
/// other handles are 7 px), and the black "+" drawn inside it.
pub(super) const OVERFLOW_HANDLE: f32 = 11.0;
pub(super) const OVERFLOW_PLUS: Stroke = Stroke { width: 1.0, color: Color32::BLACK };

/// The text caret, drawn in order: a white halo, then a solid black line inside it, so it reads on
/// any background (#6085). The black line used to be a 0.75 pt hairline inside a white one, which
/// on a white background left only a faint grey line.
pub(super) const CARET_STROKES: [Stroke; 2] = [Stroke { width: 3.0, color: Color32::WHITE }, Stroke { width: 1.0, color: Color32::BLACK }];

/// The Type Mask tools while typing: Quick Mask's colour over the canvas with the glyphs clear,
/// as Photoshop shows the selection-to-be. The glyphs come from the hidden working layer's pixels.
fn draw_mask_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, id: LayerId) {
    let Some(st) = app.session.active() else { return };
    let canvas = st.doc.bounds();
    let opts = app.session.quick_mask_options;
    let rgb = opts.color.to_rgb();
    let tint = |a: f32| {
        let a = (a * opts.opacity).clamp(0.0, 1.0);
        let q = |v: f32| (v.clamp(0.0, 1.0) * a * 255.0 + 0.5) as u8;
        Color32::from_rgba_premultiplied(q(rgb[0]), q(rgb[1]), q(rgb[2]), (a * 255.0 + 0.5) as u8)
    };
    let corners = |r: photocraft_geom::Rect| -> Vec<Pos2> {
        let (x0, y0, x1, y1) = (r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32);
        vec![xf.to_screen(x0, y0), xf.to_screen(x1, y0), xf.to_screen(x1, y1), xf.to_screen(x0, y1)]
    };
    let surface = text_layer(&st.doc, id).and_then(|t| t.cache.as_ref());
    let area = surface.map(|s| s.content_bounds().intersect(&canvas)).unwrap_or(photocraft_geom::Rect::EMPTY);
    // One unfeathered mesh, so the bands and the text's box meet without seams.
    let mut bands = egui::Mesh::default();
    let mut band = |x0: i32, y0: i32, x1: i32, y1: i32| {
        let r = photocraft_geom::Rect { x0, y0, x1, y1 };
        if !r.is_empty() {
            let base = bands.vertices.len() as u32;
            for p in corners(r) {
                bands.colored_vertex(p, tint(1.0));
            }
            bands.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    };
    let Some(surface) = surface.filter(|_| !area.is_empty()) else {
        band(canvas.x0, canvas.y0, canvas.x1, canvas.y1);
        painter.add(egui::Shape::mesh(bands));
        return;
    };
    band(canvas.x0, canvas.y0, canvas.x1, area.y0);
    band(canvas.x0, area.y1, canvas.x1, canvas.y1);
    band(canvas.x0, area.y0, area.x0, area.y1);
    band(area.x1, area.y0, canvas.x1, area.y1);
    painter.add(egui::Shape::mesh(bands));
    // The text's box: a texture of the tint where there is no glyph, cached per revision.
    let ctx = painter.ctx();
    let key = (st.doc.id.0, st.revision, id.0);
    let cache = egui::Id::unique("pc-type-mask-overlay");
    let tex = ctx.data(|d| d.get_temp::<((u64, u64, u64), egui::TextureHandle)>(cache)).filter(|(k, _)| *k == key).map(|(_, t)| t);
    let tex = tex.unwrap_or_else(|| {
        let max = (ctx.input(|i| i.max_texture_side) as u32).clamp(256, 4096);
        let factor = area.width().max(area.height()).div_ceil(max).max(1);
        let (w, h) = (area.width().div_ceil(factor) as usize, area.height().div_ceil(factor) as usize);
        let mut row = vec![[0.0f32; 4]; area.width() as usize];
        let mut px = Vec::with_capacity(w * h);
        for y in (area.y0..area.y1).step_by(factor as usize) {
            surface.read_rgba_into(photocraft_geom::Rect { x0: area.x0, y0: y, x1: area.x1, y1: y + 1 }, &mut row);
            px.extend(row.iter().step_by(factor as usize).take(w).map(|p| tint(1.0 - p[3])));
        }
        px.resize(w * h, tint(1.0));
        let img = egui::ColorImage::new([w, h], px);
        let t = ctx.load_texture("type-mask-overlay", img, egui::TextureOptions::LINEAR);
        ctx.data_mut(|d| d.insert_temp(cache, (key, t.clone())));
        t
    });
    let c = corners(area);
    let mut mesh = egui::Mesh::with_texture(tex.id());
    for (p, uv) in c.iter().zip([egui::pos2(0.0, 0.0), egui::pos2(1.0, 0.0), egui::pos2(1.0, 1.0), egui::pos2(0.0, 1.0)]) {
        mesh.vertices.push(egui::epaint::Vertex { pos: *p, uv, color: Color32::WHITE });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}
