use super::defaults::default_style_params;
use super::editing::{begin_edit, commit_with};
use super::layout::{hit_layer, hit_offset, layout, to_text};
use super::{BoxResize, LayerId, PLACEHOLDER, PhotocraftApp, Point, TextEdit, TextLayout, active_doc_id, byte_of, current_text, json, session_key, text_layer};

/// Smallest paragraph box a handle drag leaves, in text-space px.
pub(super) const MIN_BOX: f32 = 4.0;

/// Which box edges handle `i` moves: (left, right, top, bottom). The order matches the overlay.
pub(super) fn handle_sides(i: u8) -> (bool, bool, bool, bool) {
    match i {
        0 => (true, false, true, false),
        1 => (false, true, true, false),
        2 => (false, true, false, true),
        3 => (true, false, false, true),
        4 => (false, false, true, false),
        5 => (false, true, false, false),
        6 => (false, false, false, true),
        _ => (true, false, false, false),
    }
}

pub(crate) fn box_shape(app: &PhotocraftApp, id: LayerId) -> Option<(f32, f32, f32, f32)> {
    match app.session.active().and_then(|s| text_layer(&s.doc, id).map(|t| t.shape.clone()))? {
        photocraft_doc::text::TextShape::Box { x, y, width, height } => Some((x, y, width, height)),
        _ => None,
    }
}

/// The paragraph-box handle under the document point, if the layer is paragraph text.
pub(crate) fn box_handle_at(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> Option<u8> {
    let (bx, by, w, h) = box_shape(app, id)?;
    let (_, aff, _) = layout(app, id)?;
    let (r, b) = (bx + w, by + h);
    let (mx, my) = (bx + w / 2.0, by + h / 2.0);
    let spots = [(bx, by), (r, by), (r, b), (bx, b), (mx, by), (r, my), (mx, b), (bx, my)];
    let tol = f64::from(6.0 / app.point_zoom().max(0.01));
    spots
        .iter()
        .position(|&(sx, sy)| {
            let p = aff.apply(Point::new(f64::from(sx), f64::from(sy)));
            (p.x - x).abs() <= tol && (p.y - y).abs() <= tol
        })
        .map(|i| i as u8)
}

/// Drag the handle of `r` (the box and transform snapshotted at the press) to the document
/// point: the dragged edges follow it, the others stay put. With ⌥ the opposite edges mirror
/// the drag about the centre the box started with, so the block grows symmetrically instead of
/// from one side — pressing or releasing ⌥ mid-drag switches that on and off as if it had been
/// held from the start.
fn resize_box(app: &mut PhotocraftApp, ed: &TextEdit, r: BoxResize, x: f64, y: f64, alt: bool) {
    let (px, py) = to_text(&r.aff, x, y);
    let (l, rw, t, b) = handle_sides(r.handle);
    let (bx, by, w, h) = (r.shape[0], r.shape[1], r.shape[2], r.shape[3]);
    let (mut x0, mut y0, mut x1, mut y1) = (bx, by, bx + w, by + h);
    // The centre the ⌥ mirror turns about: the box as it stood at the press (every move
    // recomputes from that snapshot, so the centre holds for the whole drag).
    let (cx, cy) = (bx + w / 2.0, by + h / 2.0);
    if l {
        x0 = px.min(x1 - MIN_BOX);
    }
    if rw {
        x1 = px.max(x0 + MIN_BOX);
    }
    if t {
        y0 = py.min(y1 - MIN_BOX);
    }
    if b {
        y1 = py.max(y0 + MIN_BOX);
    }
    if alt {
        // The mirror keeps the centre fixed: the dragged edge clamps at MIN_BOX/2 from it, and
        // the opposite edge follows, so the block never inverts below the minimum.
        if l {
            x0 = px.min(cx - MIN_BOX / 2.0);
            x1 = 2.0 * cx - x0;
        }
        if rw {
            x1 = px.max(cx + MIN_BOX / 2.0);
            x0 = 2.0 * cx - x1;
        }
        if t {
            y0 = py.min(cy - MIN_BOX / 2.0);
            y1 = 2.0 * cy - y0;
        }
        if b {
            y1 = py.max(cy + MIN_BOX / 2.0);
            y0 = 2.0 * cy - y1;
        }
    }
    // type.edit places the box's top-left at the given document point.
    let o = r.aff.apply(Point::new(f64::from(x0), f64::from(y0)));
    let _ = app.run("type.edit", json!({"layer": ed.layer, "box": [o.x, o.y, x1 - x0, y1 - y0], "coalesce": ed.session}));
}

/// True when document point (`x`, `y`) lies inside the paragraph box of type layer `id` (false
/// for point text).
pub(crate) fn inside_box(app: &mut PhotocraftApp, id: LayerId, x: f64, y: f64) -> bool {
    let Some((bx, by, w, h)) = box_shape(app, id) else { return false };
    let Some((_, aff, _)) = layout(app, id) else { return false };
    let (tx, ty) = to_text(&aff, x, y);
    tx >= bx && tx <= bx + w && ty >= by && ty <= by + h
}

/// Pointer down with the Type tool. Returns true when the press was consumed (no box drag).
pub fn pointer_down(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) -> bool {
    if let Some(ed) = app.ui.text_edit.clone() {
        let id = LayerId(ed.layer);
        if let Some(i) = box_handle_at(app, id, x, y) {
            // Snapshot the box and transform now: every move recomputes from them, so ⌥ held
            // or released mid-drag mirrors about the centre the drag started with.
            let snap = box_shape(app, id).zip(layout(app, id)).map(|((bx, by, w, h), (_, aff, _))| BoxResize { handle: i, shape: [bx, by, w, h], aff });
            if let (Some(snap), Some(e)) = (snap, app.ui.text_edit.as_mut()) {
                e.resize = Some(snap);
            }
            return true;
        }
        // Photoshop: anywhere inside the paragraph box places the caret, even off the glyphs
        // (left of a line's start, below the last line); only a click outside it commits.
        if hit_layer(app, x, y) == Some(id) || inside_box(app, id, x, y) {
            let off = hit_offset(app, id, x, y);
            if let Some(e) = app.ui.text_edit.as_mut() {
                e.caret = off;
                if !mods.shift {
                    e.anchor = off;
                }
                e.dragging = true;
            }
            return true;
        }
        // ⇧/⌥ on that click combine a Type Mask with the selection.
        commit_with(app, mods);
        return true; // Photoshop: a click outside commits without starting new text
    }
    // The Type Mask tools always start new text: existing type layers are just pixels to them;
    // so does a ⇧-click with the Type tool, as in Photoshop, to start new text over old.
    if app.ui.tool.is_type_mask() || mods.shift {
        return false;
    }
    if let Some(id) = hit_layer(app, x, y) {
        let _ = app.session.select_layer(id);
        let key = session_key(app);
        if begin_edit(app, id, &key).is_err() {
            return true;
        }
        let off = hit_offset(app, id, x, y);
        app.ui.text_edit = Some(TextEdit {
            layer: id.0,
            document: active_doc_id(app),
            caret: off,
            anchor: off,
            session: key,
            created: false,
            mask: false,
            dragging: true,
            resize: None,
            preedit: None,
        });
        app.ui.text_edit_return_tool = None;
        return true;
    }
    false
}

pub fn pointer_move(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    let Some(ed) = app.ui.text_edit.clone() else { return };
    if let Some(r) = ed.resize {
        resize_box(app, &ed, r, x, y, mods.alt);
        return;
    }
    if ed.dragging {
        let off = hit_offset(app, LayerId(ed.layer), x, y);
        if let Some(e) = app.ui.text_edit.as_mut() {
            e.caret = off;
        }
    }
}

/// What a Type tool click on a path does, like Photoshop: inside a closed path, area type;
/// on a path's outline, type on the path from that point.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PathClick {
    Area { shape: Option<u64> },
    OnPath { shape: Option<u64>, subpath: usize, start: f64 },
}

/// Arc length (px) along `sub` from its first anchor to segment `seg` at curve parameter `t`.
fn arc_length_to(sub: &photocraft_doc::Subpath, seg: usize, t: f64) -> f64 {
    const PIECES: usize = 32;
    let eval = photocraft_vector::edit::eval;
    let length = |s: &[Point; 4], upto: f64| {
        (1..=PIECES)
            .fold((0.0, eval(s, 0.0)), |(len, prev), j| {
                let q = eval(s, upto * j as f64 / PIECES as f64);
                (len + (q.x - prev.x).hypot(q.y - prev.y), q)
            })
            .0
    };
    let segs = sub.segments();
    let whole: f64 = segs.iter().take(seg).map(|s| length(s, 1.0)).sum();
    whole + segs.get(seg).map_or(0.0, |s| length(s, t.clamp(0.0, 1.0)))
}

/// The path a Type tool click at document `(x, y)` would use: on the outline (within 6 screen px)
/// of the active shape layer's path, else of the work path, type on that path; inside a closed
/// one, area type. The work path's inside counts only while the canvas draws it (selected in the
/// Paths panel, #6551): a hidden Work Path left after Load as Selection must not turn a plain
/// click into area type. None while View › Show › Target Path (or Extras) hides paths.
pub(crate) fn path_target(app: &PhotocraftApp, x: f64, y: f64) -> Option<PathClick> {
    use photocraft_vector::edit::{Hit, hit};
    if !app.ui.view.shows(app.ui.view.show.target_path) {
        return None;
    }
    let shape_path = crate::vector_ui::active_shape_path(app);
    let work = app.session.active().and_then(|s| s.doc.work_path.clone());
    let work_shown = app.ui.selected_path.as_deref() == Some("work");
    // The canvas hover runs this every frame and `hit` samples every segment, so a path with more
    // anchors than this is not a click target (the click and the pointer agree: plain point
    // text). 4096 anchors is far beyond hand-drawn paths and shapes yet keeps a frame cheap, and
    // it is the most `type.fillPath` accepts, so every area click target can be filled.
    const MAX_TARGET_KNOTS: usize = photocraft_text::area::MAX_AREA_KNOTS;
    let small = |path: &photocraft_doc::vector::Path| path.subpaths.iter().map(|s| s.knots.len()).fold(0usize, usize::saturating_add) <= MAX_TARGET_KNOTS;
    let tol = 6.0 / app.point_zoom().max(0.01) as f64;
    // Only the horizontal Type tool starts type on a path: vertical type on a path is not laid
    // out yet.
    let vertical = app.ui.tool.is_vertical_type();
    let outline = |path: &photocraft_doc::vector::Path, shape: Option<u64>| {
        let on_path = |sp: usize, k: usize, t: f64| {
            // Type on a path needs a segment to run along (`type.attachToPath` rejects one anchor).
            let sub = path.subpaths.get(sp).filter(|s| !vertical && s.knots.len() >= 2)?;
            Some(PathClick::OnPath { shape, subpath: sp, start: arc_length_to(sub, k, t) })
        };
        match hit(path, Point::new(x, y), tol, &[]) {
            Some(Hit::Segment([sp, k], t)) => on_path(sp, k, t),
            Some(Hit::Anchor([sp, k])) => on_path(sp, k, 0.0),
            _ => None,
        }
    };
    let inside = |path: &photocraft_doc::vector::Path| photocraft_text::area::AreaShape::new(path, false).is_some_and(|a| a.contains(x as f32, y as f32));
    if let Some((id, p)) = shape_path.as_ref().filter(|(_, p)| small(p)) {
        if let Some(c) = outline(p, Some(*id)) {
            return Some(c);
        }
        if inside(p) {
            return Some(PathClick::Area { shape: Some(*id) });
        }
    }
    let w = work.as_ref().filter(|w| small(w))?;
    if let Some(c) = outline(w, None) {
        return Some(c);
    }
    (work_shown && inside(w)).then_some(PathClick::Area { shape: None })
}

/// What the Type tool's pointer over document `(x, y)` shows (#218): the path click a press there
/// would make, unless the press edits the type layer under it instead (`pointer_down`: an
/// unshifted press on shown type), where Photoshop keeps the plain I-beam. The type hit is only
/// tested over a path, so a hover elsewhere lays out nothing. Nothing while a type edit is open
/// (a click then commits rather than starting new type). While the button is held it is what the
/// release makes (`pointer_up`): nothing once the drag is a text box, else the path click at the
/// press.
pub(crate) fn hover_path_target(app: &mut PhotocraftApp, x: f64, y: f64, shift: bool) -> Option<PathClick> {
    if app.ui.text_edit.is_some() {
        return None;
    }
    if let Some(start) = app.drag.as_ref().map(|d| d.start) {
        // The press already passed over editing type (`pointer_down`), so no type hit test.
        return if makes_box(app, start, [x, y]) { None } else { path_target(app, start[0], start[1]) };
    }
    let target = path_target(app, x, y)?;
    (shift || hit_layer(app, x, y).is_none()).then_some(target)
}

/// Whether a press at `start` released at `end` drags out a paragraph box rather than clicking
/// (at least 4 screen pixels both ways).
fn makes_box(app: &PhotocraftApp, start: [f64; 2], end: [f64; 2]) -> bool {
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let min = 4.0 / app.point_zoom().max(0.01) as f64;
    w >= min && h >= min
}

/// Pointer up. `rect` is the dragged box (document px) when no edit session consumed the press.
pub fn pointer_up(app: &mut PhotocraftApp, start: [f64; 2], end: [f64; 2]) {
    if let Some(e) = app.ui.text_edit.as_mut() {
        e.dragging = false;
        e.resize = None;
        app.ui.caret_blink_time = app.last_frame_time;
        return;
    }
    let (w, h) = ((end[0] - start[0]).abs(), (end[1] - start[1]).abs());
    let min = 4.0 / app.point_zoom().max(0.01) as f64;
    let o = app.ui.tool_options.clone();
    let mask = app.ui.tool.is_type_mask();
    // A click (not a drag) with the Type tool on a path: area type inside a closed path, type
    // on the path on its outline. The Type Mask tools ignore paths.
    let target = if !mask && !makes_box(app, start, end) { path_target(app, start[0], start[1]) } else { None };
    // Preferences ▸ Type: "Fill new type layers with placeholder text" (on by default). The Type
    // Mask tools start empty, as Photoshop's do.
    let text = if !mask && app.session.prefs().type_.fill_new_type_layers_with_placeholder { PLACEHOLDER } else { "" };
    let mut p = default_style_params(app);
    p["text"] = json!(text);
    p["orientation"] = json!(if app.ui.tool.is_vertical_type() { "vertical" } else { "horizontal" });
    if w >= min && h >= min {
        p["box"] = json!([start[0].min(end[0]).round(), start[1].min(end[1]).round(), w.round(), h.round()]);
    } else {
        p["x"] = json!(start[0].round());
        p["y"] = json!(start[1].round());
    }
    let key = session_key(app);
    p["coalesce"] = json!(key);
    if let Ok(v) = app.run("type.create", p)
        && let Some(id) = v.get("layer").and_then(serde_json::Value::as_u64)
    {
        // A failed follow-up leaves point text; say why instead of passing over it.
        let follow_up = match &target {
            Some(PathClick::Area { shape }) => app.run("type.fillPath", json!({"layer": id, "shape": shape, "coalesce": key})),
            Some(PathClick::OnPath { shape, subpath, start }) => {
                app.run("type.attachToPath", json!({"layer": id, "shape": shape, "subpath": subpath, "start": start, "coalesce": key}))
            }
            None => Ok(serde_json::Value::Null),
        };
        if let Err(e) = follow_up {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        if o.type_aa != "sharp" {
            let _ = app.run("type.edit", json!({"layer": id, "antialias": o.type_aa, "coalesce": key}));
        }
        // A Type Mask's text is a hidden working layer: the canvas shows the mask overlay
        // instead, and committing turns it into the selection.
        if mask {
            let _ = app.run("layer.setProps", json!({"layer": id, "visible": false, "coalesce": key}));
        }
        // Like Photoshop: the placeholder is selected, so typing replaces it.
        let n = text.chars().count();
        app.ui.caret_blink_time = app.last_frame_time;
        app.ui.text_edit = Some(TextEdit {
            layer: id,
            document: active_doc_id(app),
            caret: n,
            anchor: 0,
            session: key,
            created: true,
            mask,
            dragging: false,
            resize: None,
            preedit: None,
        });
        app.ui.text_edit_return_tool = None;
    }
}

/// Character index of the previous / next word boundary (shared with `type.navigate`).
pub(super) fn word_boundary(text: &str, from: usize, forward: bool) -> usize {
    photocraft_text::word_boundary(text, from, forward)
}

/// Most clicks in a run that select more: 2 a word, 3 a line, 4 a paragraph, 5 all text. The
/// next quick click starts over with a caret.
const MAX_CLICKS: u32 = 5;

/// Count of this click in a run of clicks at one spot. `egui_count` is egui's own count for the
/// click (1 single, 2 double, 3 triple; it never goes past 3), which already applies the
/// double-click delay and distance; `prev` is our count for the previous click (0: none). A slow
/// or distant click (egui says 1) starts a new run; after five clicks the run starts over.
pub(crate) fn next_click_count(prev: u32, egui_count: u32) -> u32 {
    match (egui_count, prev) {
        (0 | 1, _) => 1,
        (n, 0) => n.min(MAX_CLICKS),
        (_, p) => p % MAX_CLICKS + 1,
    }
}

/// The selection (anchor, caret) a run of `count` clicks makes at character `caret`: 1 a caret,
/// 2 the word under it, 3 its visual line as laid out (a column in vertical type), 4 its
/// paragraph up to the hard return, 5 all text. `line` is the laid-out line the click hit; it
/// decides at a soft wrap, where one caret offset ends a line and starts the next. Without a
/// layout a line is its paragraph. Offsets past the text are clamped, so an empty layer or a
/// stale caret never panics.
pub(crate) fn click_selection(layout: Option<&TextLayout>, text: &str, caret: usize, line: Option<usize>, count: u32) -> (usize, usize) {
    let n = text.chars().count();
    let c = caret.min(n);
    match count {
        2 => {
            let chars: Vec<char> = text.chars().collect();
            let word = |i: usize| chars.get(i).is_some_and(|ch| ch.is_alphanumeric());
            let (mut a, mut b) = (c, c);
            while a > 0 && word(a - 1) {
                a -= 1;
            }
            while word(b) {
                b += 1;
            }
            (a, b)
        }
        3 => {
            let Some(l) = layout else { return paragraph_of(text, c) };
            let ln = line.and_then(|i| l.lines.get(i)).or_else(|| l.lines.get(photocraft_text::line_index(l, byte_of(text, c))));
            ln.map_or((c, c), |ln| (photocraft_text::char_index(text, ln.range.start), photocraft_text::char_index(text, ln.range.end)))
        }
        4 => paragraph_of(text, c),
        5 => (0, n),
        _ => (c, c),
    }
}

/// Characters of the paragraph holding character `c`, without its hard return.
fn paragraph_of(text: &str, c: usize) -> (usize, usize) {
    let b = byte_of(text, c);
    let paras = photocraft_text::layout::split_paragraphs(text);
    let Some(r) = paras.iter().find(|r| b >= r.start && b < r.end).or_else(|| paras.last()) else { return (c, c) };
    let body = text.get(r.clone()).unwrap_or("").trim_end_matches(['\r', '\n']);
    let start = photocraft_text::char_index(text, r.start);
    (start, start + body.chars().count())
}

/// A click while editing type at document point (`x`, `y`), after it placed the caret there.
/// `egui_count` is egui's count for the click (see [`next_click_count`]). Like Photoshop, two
/// clicks select a word, three the line, four the paragraph and five all text.
pub fn multi_click(app: &mut PhotocraftApp, egui_count: u32, x: f64, y: f64) {
    let Some(ed) = app.ui.text_edit.clone() else {
        app.type_clicks = None;
        return;
    };
    let prev = app.type_clicks.filter(|(layer, _)| *layer == ed.layer).map_or(0, |(_, n)| n);
    let count = next_click_count(prev, egui_count);
    app.type_clicks = Some((ed.layer, count));
    if count < 2 {
        return;
    }
    let id = LayerId(ed.layer);
    let Some(text) = current_text(app, id) else { return };
    let laid = layout(app, id);
    let (caret, line) = match &laid {
        Some((l, aff, t)) if *t == text && x.is_finite() && y.is_finite() => {
            let (tx, ty) = to_text(aff, x, y);
            let (ci, line) = photocraft_text::hit_char(l, t, tx, ty);
            (ci, Some(line))
        }
        _ => (ed.caret, None),
    };
    let (a, b) = click_selection(laid.as_ref().map(|(l, _, _)| l.as_ref()), &text, caret, line, count);
    if let Some(e) = app.ui.text_edit.as_mut() {
        (e.anchor, e.caret) = (a, b);
    }
}
