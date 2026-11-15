use super::drag::{Drag, marquee_preview_px, single_line_drag_px};
use super::selection::finish_selection_drag;
use super::strokes::{
    EngineStroke, commit_brush_stroke, feed_live_stroke, live_retouch_command, offer_live_retouch, paint_target, snap_aliased_tip, stroke_params,
};
use super::textures::shown_as_document;
use super::{PhotocraftApp, Tool, json};

#[cfg(test)]
#[path = "rectangle_click_in_shape_tests.rs"]
mod rectangle_click_in_shape_tests;

pub(crate) fn finish_gesture(app: &mut PhotocraftApp, d: Drag) {
    finish(app, d, None);
}

/// [`finish_gesture`] for a pointer release. `click`: the release, before snapping, came back
/// within half a pixel of the press (`tool_input`). A selection drag that snapping pulled back onto
/// its press point is not one (#11392).
pub(crate) fn finish_release(app: &mut PhotocraftApp, d: Drag, click: bool) {
    finish(app, d, Some(click));
}

/// `click`: whether the gesture was a click, when the caller saw the release before snapping;
/// otherwise the end point decides.
fn finish(app: &mut PhotocraftApp, mut d: Drag, click: Option<bool>) {
    // Control can select a different tool and release before the next frame cancels the drag.
    // Cancel before even recording a Shift-stroke anchor, including the preview fallback paths.
    if d.tool == Tool::MixerBrush && app.ui.tool != Tool::MixerBrush {
        app.live_stroke = None;
        return;
    }
    if d.vector_owner.as_ref().is_some_and(|owner| !owner.valid(app)) {
        return;
    }
    let mut end = d.points.last().copied().unwrap_or([d.start[0], d.start[1], 1.0]);
    let (sx, sy) = snap_aliased_tip(app, d.tool, end[0], end[1]);
    end[0] = sx;
    end[1] = sy;
    if d.sel_move.is_some() {
        let Some(line) = finish_selection_release(app, &mut d, end, click) else { return };
        end = line;
    }
    // Where the next ⇧-click line starts.
    if crate::stroke_constraint::connects(d.tool)
        && let Some(st) = app.session.active()
    {
        app.last_stroke_end = Some((st.doc.id, end));
    }
    if d.tool == Tool::MixerBrush {
        // The release can carry a new endpoint, including moves deferred in this frame.
        // Feed those samples before offering the already-rendered document for commit.
        app.drag = Some(d);
        let deferred = std::mem::replace(&mut app.defer_live_stroke, false);
        feed_live_stroke(app);
        app.defer_live_stroke = deferred;
        let Some(d) = app.drag.take() else { return };
        let live = app.live_stroke.take();
        if let Some(l) = live {
            // What the stroke's thread was still painting.
            let (painted, stroke) = l.paint.finish();
            let failed = l.preview_failed || painted.iter().any(Result::is_err);
            let steps = (l.damage.len() + painted.len()) as u64;
            let (doc, key, seed) = (l.doc, l.key, l.seed);
            match stroke {
                Some(EngineStroke::Mixer(mixer)) if !failed => match mixer.prepare_commit(&mut app.session) {
                    Ok(p) => {
                        if app.run("paint.mixerBrush", p).is_ok() {
                            shown_as_document(app, doc, |k| k == 0 || (key..=key + steps).contains(&k));
                        }
                    }
                    Err(e) => {
                        app.ui.status = e.to_string();
                        app.ui.status_error = true;
                    }
                },
                // A step failed, or the thread stopped mid-step: render the whole stroke.
                stroke => {
                    drop(stroke);
                    let mut p = stroke_params(app, &d, &app.stylus.stroke_points(&d.points));
                    if let Some(seed) = seed {
                        p["seed"] = json!(seed);
                    }
                    let _ = app.run("paint.mixerBrush", p);
                }
            }
        } else {
            // No preview (a tip above its size): render the whole stroke now.
            let p = stroke_params(app, &d, &app.stylus.stroke_points(&d.points));
            let _ = app.run("paint.mixerBrush", p);
        }
        return;
    }
    // A live retouching preview ends here; the commit below replaces it (the Clone Stamp's,
    // History Brush's and sequential-dab tools' with the pixels it already drew).
    offer_live_retouch(app, &d);
    if live_retouch_command(d.tool).is_some() || crate::retouch_ui::dab_params(app, d.tool).is_some() || d.tool == Tool::ColorReplacement {
        app.live_stroke = None;
    }
    let finished = crate::eraser_ui::finish_stroke(app, d.tool, &d.points, d.line)
        || crate::retouch_ui::finish_stroke_with(app, d.tool, &d.points, d.modifiers, d.line, d.history_params.clone());
    app.session.clear_live_stroke();
    if finished {
        return;
    }
    match d.tool {
        Tool::ObjectSelection => crate::retouch_ui::finish_object_selection(app, d.start, [end[0], end[1]], d.modifiers),
        Tool::FreeformPen => {
            if let Some(owner) = d.pen_owner {
                crate::vector_ui::freeform_finish_owned(app, &d.points, d.lasso.as_ref().map_or(&[], |l| l.straight()), owner);
            }
        }
        // Photoshop: a click without dragging opens Create Rectangle / Ellipse / … instead. In Path
        // mode a click adds nothing: the dialog makes a shape layer, which Path mode never draws,
        // and the click's few pixels of travel would add a speck of a subpath to the work path.
        t if crate::vector_ui::is_shape_tool(t)
            && crate::shape_dialog::is_tool_click(t, &app.ui.tool_options, d.start, &d.points, app.point_zoom())
            && (crate::vector_ui::draws_path(&app.ui.tool_options)
                || crate::shape_dialog::click_inside_active_rectangle(app, t, d.start)
                || crate::shape_dialog::open(app, t, d.start).is_some()) => {}
        t if crate::vector_ui::is_shape_tool(t) => crate::vector_ui::finish_shape(app, t, d.start, [end[0], end[1]], d.live),
        Tool::PathSelection => crate::vector_ui::path_selection_finish(app, d.start, [end[0], end[1]]),
        Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => crate::type_tool::pointer_up(app, d.start, [end[0], end[1]]),
        Tool::Brush | Tool::Pencil | Tool::Eraser => commit_brush_stroke(app, &d),
        Tool::RectMarquee | Tool::EllipseMarquee => {
            // Exactly what the drag showed (`marquee_preview_px`); a click deselects.
            let Some([x0, y0, x1, y1]) = marquee_preview_px(&app.ui.tool_options, &d, app.point_zoom()) else {
                if app.session.is_enabled("select.deselect") {
                    let _ = app.run("select.deselect", json!({}));
                }
                return;
            };
            let mode = selection_mode(app, d.modifiers);
            let (aa, feather) = (app.ui.tool_options.anti_alias, app.ui.tool_options.feather);
            let _ = app.run("select.rect", json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0, "mode": mode, "ellipse": d.tool == Tool::EllipseMarquee, "antiAlias": aa, "feather": feather}));
        }
        // A click (or the end of a drag) selects the line under the pointer, never deselects.
        Tool::SingleRowMarquee | Tool::SingleColumnMarquee => {
            let Some([x0, y0, x1, y1]) = single_line_drag_px(app, &d) else { return };
            let mode = selection_mode(app, d.modifiers);
            let feather = app.ui.tool_options.feather;
            // `line` names the History step after the tool (Single Row / Single Column Marquee).
            let line = if d.tool == Tool::SingleRowMarquee { "row" } else { "column" };
            let _ = app.run(
                "select.rect",
                json!({"x": x0, "y": y0, "width": x1 - x0, "height": y1 - y0, "mode": mode, "line": line, "antiAlias": false, "feather": feather}),
            );
        }
        Tool::Patch if crate::retouch_ui::patch_drags_selection(app, d.start, d.modifiers) => {
            crate::retouch_ui::finish_patch(app, d.start, crate::retouch_ui::patch_end(&d, [end[0], end[1]]))
        }
        Tool::ContentAwareMove if crate::retouch_ui::patch_drags_selection(app, d.start, d.modifiers) => {
            crate::retouch_ui::finish_content_aware_move(app, d.start, [end[0], end[1]])
        }
        Tool::Lasso | Tool::Patch | Tool::ContentAwareMove => {
            let pts: Vec<[f64; 2]> = d.points.iter().map(|p| [p[0], p[1]]).collect();
            // A Content-Aware Move drag with nothing to select only says so (#12192).
            if d.tool == Tool::ContentAwareMove && crate::retouch_ui::content_aware_move_no_pixels(app, &pts) {
                return;
            }
            if pts.len() >= 3 {
                let mode = selection_mode(app, d.modifiers);
                let o = &app.ui.tool_options;
                // The Patch and Content-Aware Move tools have no Feather in their options bar.
                let feather = if d.tool == Tool::Lasso { o.feather } else { 0.0 };
                let mut params = json!({"points": pts, "mode": mode, "antiAlias": o.anti_alias, "feather": feather});
                // The Patch tool's lasso is a step of that tool in History; the Lasso tool keeps "Lasso".
                if d.tool == Tool::Patch {
                    params["historyName"] = json!("Patch Tool");
                }
                let made = app.run("select.lasso", params);
                // The lasso only outlines the patch; say that it is dragged next (#1715).
                if made.is_ok() && d.tool != Tool::Lasso {
                    crate::retouch_ui::patch_hint(app);
                }
            } else if (d.tool != Tool::Lasso || selection_mode(app, d.modifiers) == "replace") && app.session.is_enabled("select.deselect") {
                let _ = app.run("select.deselect", json!({}));
            }
        }
        Tool::Gradient if (end[0] - d.start[0]).abs() + (end[1] - d.start[1]).abs() >= 2.0 => {
            let o = app.ui.tool_options.clone();
            let _ = app.run(
                "paint.gradient",
                json!({"from": [d.start[0], d.start[1]], "to": [end[0], end[1]], "style": o.gradient_style, "reverse": o.gradient_reverse, "dither": o.gradient_dither, "opacity": o.fill_opacity, "mode": o.gradient_blend_mode.label(), "target": paint_target(app)}),
            );
        }
        Tool::Move => {
            let (dx, dy) = ((end[0] - d.start[0]).round(), (end[1] - d.start[1]).round());
            crate::move_ui::finish(app, dx, dy);
        }
        _ => {}
    }
}

/// The release of selection drag `d` at `end` (`click`: see [`finish`]). A Single Row / Single
/// Column click inside the selection or on a floating piece selects a line, as it does elsewhere,
/// instead of deselecting or dropping as other marquees do (#11392): the point it selects at, for
/// the tool's own release to go on with. `None`: the release is done. A click ends here unsnapped
/// (`tool_input`), so it is never a move; the line snaps as a click outside the selection does (its
/// press began as a move, which keeps the press point).
fn finish_selection_release(app: &mut PhotocraftApp, d: &mut Drag, end: [f64; 3], click: Option<bool>) -> Option<[f64; 3]> {
    let floating = d.sel_move?;
    let unmoved = (end[0] - d.start[0]).round() == 0.0 && (end[1] - d.start[1]).round() == 0.0;
    let click = click.unwrap_or(unmoved);
    if unmoved && !click {
        // A drag that snapping pulled back where it started leaves the selection as it was. Its
        // preview of a floating piece goes too: nothing changed for it to go stale with, and a
        // Move-tool drag's release (`move_ui::finish`) is not reached. It is rebuilt as needed.
        app.move_preview = None;
        return None;
    }
    if !click || !matches!(d.tool, Tool::SingleRowMarquee | Tool::SingleColumnMarquee) {
        finish_selection_drag(app, floating, d.start, [end[0], end[1]]);
        return None;
    }
    // The piece drops where it is first, as a press off it drops it (`tool_input`).
    if floating
        && app.session.active().and_then(photocraft_engine::float_cmds::floating).is_some()
        && let Err(e) = app.run("select.drop", json!({}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
        return None;
    }
    let [x, y] = crate::snap_ui::snap_click(app, d.start, d.live);
    d.points.push([x, y, end[2]]);
    Some([x, y, end[2]])
}

/// Selection mode from the options bar, overridden by modifier keys (⇧ add, ⌥ subtract, ⇧⌥ intersect).
/// The cursor badge announces the same mode (`tool_feedback`).
pub(crate) fn selection_mode(app: &PhotocraftApp, m: egui::Modifiers) -> &'static str {
    crate::tool_feedback::document_selection_mode(app, Tool::Lasso, m)
}

/// Shift constrains a Polygonal Lasso segment to Photoshop's 45° directions.
/// Work in document coordinates so zoom, view rotation and mirroring cannot skew it.
/// A diagonal keeps the shorter of the two offsets on both axes, as Photoshop does (#11393);
/// a horizontal or vertical snap keeps the segment's length.
pub(crate) fn constrained_polygon_point(from: [f64; 2], point: [f64; 2]) -> [f64; 2] {
    let dx = point[0] - from[0];
    let dy = point[1] - from[1];
    if !dx.is_finite() || !dy.is_finite() {
        return point;
    }
    let length = dx.hypot(dy);
    if length == 0.0 {
        return point;
    }
    let octant = (dy.atan2(dx) / std::f64::consts::FRAC_PI_4).round();
    let angle = octant * std::f64::consts::FRAC_PI_4;
    if octant.rem_euclid(2.0) == 1.0 {
        let m = dx.abs().min(dy.abs());
        return [from[0] + m * angle.cos().signum(), from[1] + m * angle.sin().signum()];
    }
    [from[0] + length * angle.cos(), from[1] + length * angle.sin()]
}

/// How close (in document units) a click must land to a polygon's first vertex to close it: 8
/// points on screen, but never more than a quarter of the outline's on-screen width (at least 1
/// point), so the last corner of a small shape doesn't close it early (#6578). The width is the
/// narrower side of the smallest bounding box that fits the outline, turned to line up with one of
/// its edges: a large polygon with one short edge keeps the full 8 points (#7629), while a thin
/// strip stays narrow however it is rotated. Outlines wider than 32 points keep 8. Non-finite
/// points are ignored. Only the first vertex uses this; the last one has its own fixed reach.
pub(crate) fn polygon_close_reach(points: impl IntoIterator<Item = [f64; 2]>, point_zoom: f32) -> f64 {
    let zoom = f64::from(point_zoom.max(0.01));
    let pts: Vec<[f64; 2]> = points.into_iter().filter(|p| p.iter().all(|n| n.is_finite())).collect();
    // Edge directions to try; a freehand outline has thousands, so sample them.
    let step = pts.len().div_ceil(64).max(1);
    let mut width = f64::INFINITY;
    for i in (0..pts.len()).step_by(step) {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        let len = (b[0] - a[0]).hypot(b[1] - a[1]);
        if len <= 0.0 {
            continue;
        }
        let normal = [-(b[1] - a[1]) / len, (b[0] - a[0]) / len];
        let along = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
        let extent = |axis: [f64; 2]| {
            let (lo, hi) = pts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
                let v = p[0] * axis[0] + p[1] * axis[1];
                (lo.min(v), hi.max(v))
            });
            hi - lo
        };
        width = width.min(extent(normal).min(extent(along)));
    }
    let screen = if width.is_finite() { (width * zoom / 4.0).clamp(1.0, 8.0) } else { 8.0 };
    screen / zoom
}

/// A polygonal lasso click adds a vertex; clicking near the first vertex closes the polygon. The
/// first click fixes the selection mode; a new-selection polygon hides the old outline while it is
/// drawn, and replaces it in one history step when it closes.
pub(super) fn polygon_click(app: &mut PhotocraftApp, x: f64, y: f64, mods: egui::Modifiers) {
    polygon_cancel_stale(app);
    let close =
        app.ui.polygon.first().is_some_and(|p0| {
            app.ui.polygon.len() >= 3 && (p0[0] - x).hypot(p0[1] - y) < polygon_close_reach(app.ui.polygon.iter().copied(), app.point_zoom())
        });
    if close {
        commit_polygon(app);
        return;
    }
    if app.ui.polygon.is_empty() {
        app.ui.polygon_doc = app.session.active().map(|st| st.doc.id);
        // ⌥ with nothing selected draws freehand; there is nothing to subtract from.
        let nothing_selected = app.session.active().is_none_or(|st| st.doc.selection.is_none());
        let intent = if nothing_selected { egui::Modifiers { alt: false, ..mods } } else { mods };
        app.ui.polygon_mode = selection_mode(app, intent).into();
    }
    // Only constrain segments after the first vertex; the first click still sets selection intent.
    // Alt-drag is freehand and must not snap to an angle.
    let point = if mods.shift && !mods.alt { app.ui.polygon.last().copied().map_or([x, y], |last| constrained_polygon_point(last, [x, y])) } else { [x, y] };
    app.ui.polygon.push(point);
}

/// Remove the Polygonal Lasso's last vertex (⌫, Delete or a right-click while drawing, #1229);
/// removing the only one cancels the polygon. False when no polygon is being drawn.
pub fn polygon_retract(app: &mut PhotocraftApp) -> bool {
    if app.ui.polygon.pop().is_none() {
        return false;
    }
    if app.ui.polygon.is_empty() {
        app.ui.polygon_mode.clear();
        app.ui.polygon_doc = None;
    }
    true
}

/// The polygon in progress belongs to the document it was started on (`UiState::polygon_doc`):
/// once another document becomes active it is dropped, so its outline doesn't show on (or get
/// committed to) the other document, and the selection it would have replaced is shown again.
/// Runs before canvas input and shortcuts as well as in `PhotocraftApp::sync_views`. Like the Lasso's
/// ([`crate::lasso_ui::cancel_stale`]) and the Magnetic Lasso's ([`crate::magnetic_lasso_ui::frame`]).
pub fn polygon_cancel_stale(app: &mut PhotocraftApp) {
    let active = app.session.active().map(|st| st.doc.id);
    if app.ui.polygon.is_empty() {
        app.ui.polygon_doc = None;
    } else if app.ui.tool != Tool::PolygonLasso || app.ui.polygon_doc.is_some_and(|d| Some(d) != active) {
        app.ui.polygon.clear();
        app.ui.polygon_mode.clear();
        app.ui.polygon_doc = None;
    }
}

/// A new-selection polygonal (or magnetic) lasso is being drawn, so the selection it will replace
/// is hidden.
pub fn polygon_replaces_selection(app: &PhotocraftApp) -> bool {
    (!app.ui.polygon.is_empty() && app.ui.polygon_mode == "replace") || crate::magnetic_lasso_ui::replaces_selection(app)
}

/// Close the polygonal lasso and make the selection in the mode it started in.
pub fn commit_polygon(app: &mut PhotocraftApp) {
    polygon_cancel_stale(app);
    let pts = std::mem::take(&mut app.ui.polygon);
    let mode = std::mem::take(&mut app.ui.polygon_mode);
    app.ui.polygon_doc = None;
    if pts.len() >= 3 {
        let mode = if mode.is_empty() { selection_mode(app, egui::Modifiers::NONE).to_owned() } else { mode };
        let o = &app.ui.tool_options;
        let _ = app.run("select.lasso", json!({"points": pts, "mode": mode, "antiAlias": o.anti_alias, "feather": o.feather, "polygonal": true}));
    }
}

/// Apply the crop tool's rectangle. When `image.crop` refuses it (the error is in the status bar),
/// the frame, its turn and its box Undo stay for fixing.
pub fn commit_crop(app: &mut PhotocraftApp) {
    // Never another document's frame (#1918).
    crate::crop_ui::cancel_stale(app);
    let kept = crate::crop_ui::keep(app);
    // Committing ends the box's own Undo/Redo: from here Edit › Undo reverts the document.
    crate::crop_ui::clear_history(app);
    // ↵ during a drag ends it: the release must not bring back the old document's frame.
    app.crop.drag = None;
    let Some(r) = app.ui.crop_rect.take() else { return };
    // The crop's default mode leaves the view upright (`crop_mode`).
    crate::crop_mode::upright(app);
    // The untouched default frame around the whole canvas crops nothing (Photoshop's ↵ on it does
    // nothing); one framing the selection's bounds crops to them (#1789).
    if std::mem::take(&mut app.crop.default_frame) {
        let whole = app.session.active().map(|st| st.doc.bounds()).map(|b| [f64::from(b.x0), f64::from(b.y0), f64::from(b.x1), f64::from(b.y1)]);
        if whole.is_none_or(|b| b == r) {
            return;
        }
    }
    let (x, y) = (r[0].round(), r[1].round());
    let (w, h) = ((r[2] - r[0]).round().max(1.0), (r[3] - r[1]).round().max(1.0));
    let delete = app.ui.tool_options.crop_delete;
    let mut p = json!({"x": x, "y": y, "width": w, "height": h, "deleteCroppedPixels": delete});
    // Fill › Content-Aware Fill (#3823): what a turned or extending frame uncovers is filled from
    // its surroundings instead of the background colour.
    if app.ui.tool_options.crop_content_aware {
        p["contentAware"] = json!(true);
    }
    // A turned frame (#1792): the document turns with it, then is cropped to it. (The untouched
    // default frame above is never turned: turning it makes it a real frame.)
    let angle = crate::crop_ui::angle(app);
    app.ui.crop_angle = 0.0;
    if angle != 0.0 {
        p["angle"] = json!(angle);
    }
    // W x H x Resolution (#2443): resample to that size and set the resolution.
    crate::crop_size::commit_params(app, &mut p);
    if app.run("image.crop", p).is_err() {
        crate::crop_ui::restore(app, kept);
    } else if let Some(i) = app.session.active_index() {
        app.ui.views[i].fit_pending = true;
        crate::crop_ui::hide_frame(app);
    }
}
