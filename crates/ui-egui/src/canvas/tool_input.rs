use super::cancel_stale_vector_drag;
use super::cursors::{alt_eyedropper, sample_eyedropper};
use super::drag::Drag;
use super::gesture::{finish_gesture, polygon_click, selection_mode};
use super::selection::finish_selection_drag;
use super::strokes::{HOLD_REPEAT_MS, begin_live_stroke, feed_live_stroke, paint_target, snap_aliased_tip, stroke_params, strokes_live};
use super::{PhotocraftApp, Tool, json, multiple_color_target};

/// Abstract tool event, produced by the mouse or by automation (`ui.pointer`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ToolEvent {
    Down { x: f64, y: f64, pressure: f32 },
    Move { x: f64, y: f64, pressure: f32 },
    Up { x: f64, y: f64 },
}

/// The body of a `Move` for every tool: tracked position, ⇧ constraint, the moving layer, and so
/// on. It does not feed the live stroke, so the canvas can push a whole frame's recovered samples
/// and update the live stroke once (see `canvas_view`).
fn tool_move(app: &mut PhotocraftApp, x: f64, y: f64, pressure: f32, mods: egui::Modifiers) {
    let tool = app.active_tool();
    if tool.is_type() && app.drag.is_none() {
        crate::type_tool::pointer_move(app, x, y, mods);
    }
    if tool == Tool::Pen && mods.alt {
        crate::vector_ui::pen_break_handle(app);
    }
    if tool == Tool::Pen {
        crate::vector_ui::pen_move(app, x, y, mods.shift);
    } else if tool == Tool::CurvaturePen {
        crate::vector_ui::curvature_move(app, x, y);
    }
    let zoom = app.point_zoom();
    if let Some(d) = app.drag.as_mut().filter(|d| d.reposition) {
        d.track(mods);
        d.shift_to([x, y]);
    } else if let Some(d) = &mut app.drag {
        d.track(mods);
        // ⇧: straight 0/45/90° strokes, 45° gradient angles (stroke_constraint.rs).
        let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
        let [x, y] = crate::stroke_constraint::constrain(d.tool, &mut d.constrain, d.start, last, [x, y], mods.shift, zoom);
        // ⇧: a selection-tool drag of the selection locks to multiples of 45° (#6231). The Move
        // tool's own drags were already locked by `move_mods::filter_event`.
        let [x, y] = crate::move_mods::selection_drag_to(d, [x, y], mods.shift && tool != Tool::Move);
        let moved = d.points.last().is_none_or(|p| (p[0] - x).abs() + (p[1] - y).abs() > 0.25);
        // A held pen repeats its point now and then, so the stroke's time keeps advancing.
        let held = app.stylus.last_point_ms().is_some_and(|t| app.stylus.clock_ms - t >= HOLD_REPEAT_MS)
            && app.live_stroke.as_ref().is_some_and(|l| l.stroke.wants_time());
        if moved || held {
            d.points.push([x, y, pressure as f64]);
            app.stylus.record_point();
        }
    }
}

fn preflight_tool_event(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) -> Option<(bool, egui::Modifiers)> {
    // A press anywhere but on a guide deselects the selected guide (`guide_select`).
    if let ToolEvent::Down { x, y, .. } = ev
        && app.ui.selected_guide.is_some()
        && crate::rulers::guide_at(app, x, y).is_none()
    {
        crate::guide_select::clear(app);
    }
    if crate::symmetry_ui::pointer(app, ev, crate::workspace_ui::sticky_mods(app, mods)) {
        return None;
    }
    // An Alt+right-drag armed for this press (`paint_mouse`): taken before anything else can
    // consume the event, so it never outlives the press it was armed for (#297).
    let armed = std::mem::take(&mut app.brush_resize_armed);
    crate::transform_tool::end_if_left(app);
    let mods = crate::workspace_ui::sticky_mods(app, mods);
    if app.ui.transform.is_none() && crate::type_transform::pointer(app, ev, mods) {
        return None;
    }
    // Content-Aware Move's Transform On Drop box takes every press until ↩ or Esc.
    if crate::cam_transform::pointer(app, ev, mods) {
        return None;
    }
    // A Mixer Brush press that loaded paint stays a sample press until it is released, with ⌥
    // let go or not (`mixer_sampling`; `canvas_view` holds back egui's later drag start too).
    match ev {
        ToolEvent::Down { .. } => app.mixer_sampling = false,
        ToolEvent::Up { .. } if app.mixer_sampling => {
            app.mixer_sampling = false;
            return None;
        }
        _ if app.mixer_sampling => return None,
        _ => {}
    }
    // The tool in use, so a held temporary tool (⌘ = Move) with ⌥ doesn't load paint.
    if app.active_tool() == Tool::MixerBrush && mods.alt && !mods.ctrl && app.drag.is_none() && !armed && app.brush_resize.is_none() {
        if let ToolEvent::Down { x, y, pressure } = ev {
            app.mixer_sampling = true;
            let pen = app.stylus.sample().unwrap_or_default();
            let p = json!({"points":[[x,y,pressure,pen.tilt_x,pen.tilt_y,pen.rotation]],"target":paint_target(app)});
            if let Err(e) = app.run("brush.mixer.sample", p) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        return None;
    }
    Some((armed, mods))
}

fn filter_event_before_tool(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers, armed: bool) -> Option<(ToolEvent, ToolEvent, egui::Modifiers)> {
    // View › Snap / Snap To and smart guides (snap_ui.rs).
    let raw = ev;
    // The Move tool's marquee selecting layers takes its moves and release unsnapped (move_box.rs).
    if crate::move_box::pointer(app, raw) {
        return None;
    }
    // A press anywhere but on the floating piece (or with ⇧ / ⌥, to draw) drops it first; the
    // Move tool (⌘ with a painting tool too, `event_tool`) drags it from anywhere.
    if let ToolEvent::Down { x, y, .. } = raw
        && app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some())
        && !crate::move_ui::moves_selected_pixels_with(app, event_tool(app, mods))
        && selection_drag_kind(app, app.ui.tool, [x, y], mods) != Some(true)
    {
        let _ = app.run("select.drop", json!({}));
    }
    let ev = crate::snap_ui::filter_event(app, ev, mods);
    // Free Transform picks the handle under the press where the user clicked: the snapped press
    // can land outside the corner's grab area and turn a corner drag into a move of the whole box.
    let transform_ev = if matches!(raw, ToolEvent::Down { .. }) { raw } else { ev };
    if crate::transform_tool::pointer(app, transform_ev, mods) {
        return None;
    }
    if crate::distort_ui::pointer(app, ev, mods) {
        return None;
    }
    if crate::perspective_crop_ui::pointer(app, ev) {
        return None;
    }
    // Window › Modifier Keys: sticky Shift/⌘/⌥ act as held keys.
    let mods = crate::workspace_ui::sticky_mods(app, mods);
    // ⌘⌥⌃-click with any tool selects the topmost layer with pixels there (quick_pick.rs); it
    // must run before the ⌃⌥ brush resize below, which the same keys would trigger.
    if crate::quick_pick::pointer(app, ev, mods) {
        return None;
    }
    // Control+Alt-drag or Alt+right-drag with a painting tool resizes the brush instead of
    // painting (#231, #297).
    if crate::brush_resize::pointer(app, ev, mods, armed) {
        return None;
    }
    // ⌥ (Alt) with a painting tool is the Eyedropper for that press.
    if alt_eyedropper(app, ev, mods) {
        return None;
    }
    // Move tool: ⇧ locks the axis, ⌥ duplicates (move_mods.rs).
    let ev = crate::move_mods::filter_event(app, ev, mods);
    // A live shape's corner radius widgets (Path Selection, the shape tools, Move with its
    // transform controls). Unsnapped, as Free Transform's handles: snapping would pull the press
    // off a widget onto a nearby vertex or guide, and the shape tool would draw a new shape.
    if crate::corner_radius::pointer(app, raw, mods) {
        return None;
    }
    // Direct Selection, and the Pen's ⌘ (Direct Selection) and ⌥ (Convert Point) modes (#790).
    if crate::direct_select::pointer(app, ev, mods) {
        return None;
    }
    // Add / Delete Anchor Point tools (#1044).
    if crate::anchor_tools::pointer(app, ev, mods) {
        return None;
    }
    // Ruler, Count and Note tools.
    if crate::analysis_ui::pointer(app, ev, mods) {
        return None;
    }
    // Slice and Slice Select tools.
    if crate::slice_ui::pointer(app, ev, mods) {
        return None;
    }
    // Frame tool: draw rectangular and elliptical frames.
    if crate::frame_tool::pointer(app, ev, mods) {
        return None;
    }
    // Artboard tool: draw, select, move and resize boards.
    if crate::artboard_tool::pointer(app, ev, mods) {
        return None;
    }
    // Crop tool: draw, move and resize the frame.
    if crate::crop_ui::pointer(app, ev, mods) {
        return None;
    }
    // Gradient tool, live mode: draw and edit Gradient Fill layers.
    if crate::gradient_ui::pointer(app, ev, mods) {
        return None;
    }
    // ⌘ over a guide takes the guide (#2690, #6288); the guide drag below moves it.
    if let ToolEvent::Down { x, y, .. } = raw
        && let Some((vertical, i)) = command_guide_at(app, app.active_tool(), [x, y], mods)
    {
        let mut guide = crate::rulers::GuideDrag::existing(vertical, i, [x, y]);
        guide.track([x, y], mods.alt);
        app.guide_drag = Some(guide);
        crate::snap_ui::begin_guide(app, [x, y]);
        return None;
    }
    if crate::lasso_ui::pointer(app, ev, mods) {
        return None;
    }
    // Magnetic Lasso: fastening points and the border following the pointer.
    if crate::magnetic_lasso_ui::pointer(app, ev, mods) {
        return None;
    }
    Some((raw, ev, mods))
}

fn handle_move_pointer(app: &mut PhotocraftApp, ev: ToolEvent, tool: Tool, mods: egui::Modifiers) -> bool {
    match ev {
        ToolEvent::Down { x, y, pressure } if tool == Tool::Move => {
            if let Some((vertical, i)) = crate::rulers::guide_at(app, x, y) {
                // ⌥-click turns the guide, through the point clicked, as in Photoshop.
                if mods.alt && !mods.shift && !mods.command {
                    crate::guide_select::flip(app, vertical, i, [x, y]);
                    return true;
                }
                let mut guide = crate::rulers::GuideDrag::existing(vertical, i, [x, y]);
                guide.track([x, y], mods.alt);
                app.guide_drag = Some(guide);
                return true;
            }
            if multiple_color_target(app) {
                app.ui.status = "Move and transforms on several colour channels are not implemented yet".into();
                return true;
            }
            // Auto-Select (or ⌘-click while it is off) picks the layer under the pointer first
            // (not when the selected pixels move: those are the active layer's).
            if !crate::move_ui::moves_selected_pixels_with(app, tool) && app.ui.tool_options.move_auto_select != mods.command {
                let target = app.ui.tool_options.move_target.clone();
                let mode = if mods.shift { "add" } else { "replace" };
                let before = app.session.active().map(|st| st.selected_layers());
                // A locked layer under the pointer (the Background) can't move. Photoshop draws
                // the layer marquee instead of reporting the lock (#4058). Probe without
                // selecting so a locked hit doesn't become the selection first.
                let probe = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "select": false}));
                let locked_hit = probe.as_ref().ok().and_then(|v| v["layer"].as_u64()).is_some_and(|id| {
                    app.session.active().is_some_and(|st| {
                        let id = photocraft_doc::LayerId(id);
                        let locks = st.doc.effective_locks(id);
                        locks.position || locks.all || st.doc.is_background(id)
                    })
                });
                if locked_hit || (probe.as_ref().is_ok_and(|v| v["layer"].is_null()) && crate::move_box::empty_at(app, [x, y])) {
                    if locked_hit {
                        // A click still selects the locked layer, as in Photoshop; a drag's
                        // marquee then replaces that selection on release.
                        let _ = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "mode": mode}));
                    }
                    crate::move_box::begin(app, [x, y], mods.shift);
                    return true;
                }
                let picked = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "mode": mode}));
                // No pixels there at all (an empty spot, or off the canvas): drag a marquee that
                // selects the layers it touches instead (move_box.rs).
                if picked.is_ok_and(|v| v["layer"].is_null()) && crate::move_box::empty_at(app, [x, y]) {
                    crate::move_box::begin(app, [x, y], mods.shift);
                    return true;
                }
                // Snapping (and the drag's box) started before the pick: point it at what moves.
                if app.session.active().map(|st| st.selected_layers()) != before {
                    crate::snap_ui::retarget_move(app, [x, y], mods);
                }
            }
            // A locked layer: no drag, and Photoshop's message once the pointer moves (`move_lock`).
            if crate::move_lock::blocked(app, tool, [x, y], mods) {
                app.move_blocked = true;
                return true;
            }
            // With a selection: cut the selected pixels (⌥ copies them) and drag them as a floating
            // piece, from anywhere, as a marquee ⌘-drag does.
            if crate::move_ui::moves_selected_pixels_with(app, tool) {
                if crate::move_ui::float_selected(app, mods.alt, 0.0, 0.0) {
                    let mut d = Drag::new(tool, [x, y], vec![[x, y, pressure as f64]], mods, false);
                    d.sel_move = Some(true);
                    app.drag = Some(d);
                }
                return true;
            }
        }
        ToolEvent::Move { x, y, .. } => {
            if let Some(d) = app.guide_drag.as_mut().filter(|d| d.index.is_some()) {
                d.track([x, y], mods.alt);
                return true;
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some(mut d) = app.guide_drag.filter(|d| d.index.is_some()) {
                app.guide_drag = None;
                d.track([x, y], mods.alt);
                crate::rulers::finish_drag(app, d);
                return true;
            }
        }
        _ => {}
    }
    false
}

fn consumes_cancelled_vector_event(app: &mut PhotocraftApp, ev: ToolEvent) -> bool {
    cancel_stale_vector_drag(app);
    if app.cancelled_vector_drag {
        match ev {
            ToolEvent::Down { .. } => app.cancelled_vector_drag = false,
            ToolEvent::Move { .. } => return true,
            ToolEvent::Up { .. } => {
                app.cancelled_vector_drag = false;
                return true;
            }
        }
    }
    false
}

fn handle_eyedropper_event(app: &mut PhotocraftApp, tool: Tool, ev: ToolEvent, mods: egui::Modifiers) -> bool {
    if tool != Tool::Eyedropper {
        return false;
    }
    match ev {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } => sample_eyedropper(app, x, y, mods),
        ToolEvent::Up { .. } => {}
    }
    true
}

/// Tool state machine. Shared by mouse input and automation.
pub fn tool_event(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    if consumes_cancelled_vector_event(app, ev) {
        return;
    }
    let Some((armed, mods)) = preflight_tool_event(app, ev, mods) else { return };
    let Some((raw, ev, mods)) = filter_event_before_tool(app, ev, mods, armed) else { return };
    let tool = event_tool(app, mods);
    if handle_eyedropper_event(app, tool, ev, mods) {
        return;
    }
    // A locked layer (the Background without a selection, say): no drag, and Photoshop's message
    // once the pointer moves (a click says nothing). ⌘ with a selection tool here (#896); the Move
    // tool below, once Auto-Select has picked the layer.
    match ev {
        ToolEvent::Down { x, y, .. } if tool != Tool::Move && app.ui.transform.is_none() && crate::move_lock::blocked(app, tool, [x, y], mods) => {
            app.move_blocked = true;
            return;
        }
        ToolEvent::Move { .. } if app.move_blocked => {
            app.move_blocked = false;
            crate::move_lock::prompt(app);
            return;
        }
        ToolEvent::Up { .. } if app.move_blocked => {
            app.move_blocked = false;
            return;
        }
        _ => {}
    }
    if handle_move_pointer(app, ev, tool, mods) {
        return;
    }

    match ev {
        ToolEvent::Down { x, y, pressure } => {
            // Painting a type, shape, Smart Object or fill layer asks to rasterize it first
            // (⌥-click with the Clone Stamp or Healing Brush only sets the source).
            let sets_source = matches!(tool, Tool::CloneStamp | Tool::Healing) && mods.alt;
            // Painting on or moving a hidden layer is refused at the press, as in Photoshop (#571).
            if !sets_source && let Some(why) = hidden_target(app, tool) {
                app.ui.status = why.into();
                app.ui.status_error = true;
                return;
            }
            if !sets_source && crate::rasterize_prompt::intercept(app, tool, x, y, pressure) {
                return;
            }
            // ⌘ with a selection tool is the Move tool for the drag: outside the selection (or
            // without one) it moves the whole layer, ⌘⌥ a duplicate of it. It never combines
            // selections.
            if command_moves_layer(app, tool, [x, y], mods) {
                if mods.alt
                    && let Err(e) = app.run("layer.duplicate", json!({"inPlace": true}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    return;
                }
                app.drag = Some(Drag::new(Tool::Move, [x, y], vec![[x, y, pressure as f64]], mods, false));
                return;
            }
            // Marquee / lasso inside the selection: drag the outline, or ⌘-drag to cut the selected
            // pixels into a floating piece (`select.float`) and drag that.
            if let Some(cut) = selection_drag_kind(app, tool, [x, y], mods) {
                // ⌘⌥ on a floating piece leaves it where it is and drags a copy of it.
                if cut
                    && mods.alt
                    && mods.command
                    && app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some())
                    && let Err(e) = app.run("select.drop", json!({}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    return;
                }
                if cut
                    && app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_none())
                    && let Err(e) = app.run("select.float", json!({"dx": 0, "dy": 0, "copy": mods.alt}))
                {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    return;
                }
                crate::snap_ui::follow_cut(app);
                let mut d = Drag::new(tool, [x, y], vec![[x, y, pressure as f64]], mods, false);
                d.sel_move = Some(cut);
                app.drag = Some(d);
                return;
            }
            match tool {
                Tool::PathSelection => {
                    let tol = 6.0 / f64::from(app.point_zoom().max(0.01));
                    if let Some(st) = app.session.active()
                        && let Some(id) = crate::vector_ui::pick_shape_at(&st.doc, [x, y], tol)
                    {
                        let _ = app.run("layer.select", json!({"layer": id.0}));
                    }
                    // The component pressed is selected, so the drag moves it (and the others selected).
                    crate::path_components::press(app, [x, y], mods.shift);
                }
                Tool::Pen => {
                    if !(mods.alt && crate::vector_ui::pen_grab_handle(app, x, y)) {
                        crate::vector_ui::pen_down(app, x, y, mods.shift);
                    }
                    return;
                }
                Tool::CurvaturePen => {
                    crate::vector_ui::curvature_down(app, x, y);
                    return;
                }
                Tool::CloneStamp | Tool::Healing if mods.alt => {
                    crate::retouch_ui::set_source(app, x, y);
                    return;
                }
                Tool::MagicWand => {
                    let o = app.ui.tool_options.clone();
                    let mode = selection_mode(app, mods);
                    let _ = app.run("select.magicWand", json!({"x": x.floor(), "y": y.floor(), "tolerance": o.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "sampleAllLayers": o.sample_all.magic_wand, "mode": mode}));
                    return;
                }
                Tool::MagicEraser => {
                    crate::eraser_ui::click(app, tool, x, y);
                    return;
                }
                Tool::PaintBucket => {
                    let o = app.ui.tool_options.clone();
                    let contents = if o.bucket_fill_pattern { "pattern" } else { "foreground" };
                    let mode = if o.bucket_clear { "clear" } else { o.bucket_blend_mode.label() };
                    let _ = app.run("paint.bucket", json!({"x": x.floor(), "y": y.floor(), "tolerance": o.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "sampleAllLayers": o.sample_all.paint_bucket, "opacity": o.fill_opacity, "contents": contents, "mode": mode, "target": paint_target(app)}));
                    return;
                }
                Tool::RedEye => {
                    let o = app.ui.tool_options.clone();
                    let _ = app.run(
                        "paint.redEye",
                        json!({
                            "x": x.floor(),
                            "y": y.floor(),
                            "pupilSize": o.red_eye_pupil_size.clamp(1.0, 100.0),
                            "darken": o.red_eye_darken.clamp(0.0, 100.0),
                            "target": paint_target(app),
                        }),
                    );
                    return;
                }
                Tool::PolygonLasso => {
                    polygon_click(app, x, y, mods);
                    return;
                }
                // The Type tools pick the box handle or glyph under the press where the user
                // clicked: the snapped press can land off an edge handle's grab area (#6819).
                Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask
                    if {
                        let [px, py] = if let ToolEvent::Down { x, y, .. } = raw { [x, y] } else { [x, y] };
                        crate::type_tool::pointer_down(app, px, py, mods)
                    } =>
                {
                    return;
                }
                _ => {}
            }
            crate::paint_mouse::sync_tool_brush(app);
            // Clear mode (`~` held, or chosen in the options bar) erases with the tool's own brush.
            let clear = crate::paint_mouse::clears(app, tool);
            let erase = std::mem::take(&mut app.secondary_erase) || tool == Tool::Eraser || clear;
            // A 1 px pencil line depends on the pixels, not where inside the pixel the click
            // landed (#4071). Snap both ends of the stroke to the aliased tip's grid.
            let (x, y) = snap_aliased_tip(app, tool, x, y);
            // ⇧-click connects to the last endpoint, including its pen pressure (#2827), so
            // the renderer interpolates size and opacity across the segment.
            let active = app.session.active().map(|st| st.doc.id);
            let from = app.last_stroke_end.filter(|(doc, _)| mods.shift && crate::stroke_constraint::connects(tool) && Some(*doc) == active).map(|(_, p)| {
                let (sx, sy) = snap_aliased_tip(app, tool, p[0], p[1]);
                [sx, sy, p[2]]
            });
            let mut points = vec![[x, y, pressure as f64]];
            if let Some(p) = from {
                points.insert(0, p);
            }
            // ⌥ flips Dodge/Burn and Blur/Sharpen for this stroke.
            let tool = crate::retouch_ui::alt_flipped(tool, mods.alt);
            let mut drag = Drag { clear, ..Drag::new(tool, from.map_or([x, y], |p| [p[0], p[1]]), points, mods, erase) };
            drag.capture_vector_owners(app);
            app.drag = Some(drag);
            app.trail = None;
            app.stylus.begin_stroke();
            if from.is_some() {
                app.stylus.record_point();
            }
            app.live_stroke = if strokes_live(tool) { begin_live_stroke(app) } else { None };
            if tool == Tool::MixerBrush && app.live_stroke.is_none() {
                use photocraft_engine::brush_cmds::mixer_live::LiveMixer;
                let p = app.drag.as_ref().map(|d| stroke_params(app, d, &app.stylus.stroke_points(&d.points)));
                // A tip above the preview size keeps the drag: `finish_gesture` renders it on release.
                // Otherwise the first dab itself is invalid. Do not queue an unexpected release edit.
                if let Some(p) = p
                    && !LiveMixer::defers_to_release(&app.session, &p).is_ok_and(|deferred| deferred)
                {
                    if let Err(e) = LiveMixer::begin(&app.session, &p) {
                        app.ui.status = e.to_string();
                        app.ui.status_error = true;
                    }
                    app.drag = None;
                }
            } else if tool == Tool::MixerBrush {
                feed_live_stroke(app);
            }
        }
        ToolEvent::Move { x, y, pressure } => {
            // Polygonal Lasso: ⌥ held while the button is down draws freehand into the polygon;
            // releasing ⌥ goes back to straight segments (the polygon stays open).
            if tool == Tool::PolygonLasso && mods.alt && app.drag.is_none() && !app.ui.polygon.is_empty() {
                if app.ui.polygon.last().is_none_or(|l| (l[0] - x).hypot(l[1] - y) >= 1.0) {
                    app.ui.polygon.push([x, y]);
                }
                return;
            }
            tool_move(app, x, y, pressure, mods);
            feed_live_stroke(app);
        }
        ToolEvent::Up { x, y } => {
            if tool.is_type()
                && let Some(e) = app.ui.text_edit.as_mut()
            {
                e.dragging = false;
                e.resize = None;
                app.ui.caret_blink_time = app.last_frame_time;
            }
            if tool == Tool::Pen {
                crate::vector_ui::pen_up(app);
            } else if tool == Tool::CurvaturePen {
                crate::vector_ui::curvature_up(app);
            }
            let zoom = app.point_zoom();
            let Some(mut d) = app.drag.take() else { return };
            d.track(mods);
            if d.reposition {
                d.shift_to([x, y]);
            }
            let last = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
            let [x, y] = crate::stroke_constraint::constrain(d.tool, &mut d.constrain, d.start, last, [x, y], mods.shift, zoom);
            let [x, y] = crate::move_mods::selection_drag_to(&mut d, [x, y], mods.shift && app.active_tool() != Tool::Move);
            // A Move-tool click (released where it was pressed) selects, it never moves: snapping
            // the release point would otherwise nudge the layer onto a nearby edge.
            if d.tool == Tool::Move && d.points.len() < 2 && matches!(raw, ToolEvent::Up { x, y } if [x, y] == d.start) {
                app.move_preview = None;
                crate::move_mods::finish(app);
                if d.sel_move.is_some() {
                    finish_selection_drag(app, true, d.start, d.start);
                }
                return;
            }
            if d.points.last().is_none_or(|p| p[0] != x || p[1] != y) {
                d.points.push([x, y, d.points.last().map_or(1.0, |p| p[2])]);
                app.stylus.record_point();
            }
            finish_gesture(app, d);
            crate::move_mods::finish(app);
        }
    }
}

/// Why a press with `tool` must not start: it would paint on or move a hidden target layer (the
/// engine refuses the command; refusing the press keeps the live stroke from drawing first).
fn hidden_target(app: &PhotocraftApp, tool: Tool) -> Option<&'static str> {
    let cmd = match tool {
        Tool::Move => "layer.translate",
        // The live Gradient makes a new Gradient Fill layer above the target, which Photoshop
        // allows over a hidden layer; only the classic one paints the target itself.
        Tool::Gradient if !app.ui.tool_options.gradient_classic => return None,
        // Every painting command follows the same target (layer, mask or channel) as a stroke.
        t if t.is_brushlike() || matches!(t, Tool::Gradient | Tool::PaintBucket | Tool::MagicEraser) => "paint.stroke",
        _ => return None,
    };
    photocraft_engine::hidden_target::refusal(&app.session, cmd, &json!({}))
}

/// Is document point `p` inside the active document's selection?
fn inside_selection(app: &PhotocraftApp, p: [f64; 2]) -> bool {
    app.session.active().and_then(|st| st.doc.selection.as_ref()).is_some_and(|sel| covers(sel, p))
}

/// Does selection surface `sel` cover document point `p`?
fn covers(sel: &photocraft_raster::Surface, p: [f64; 2]) -> bool {
    let mut v = [0.0f32];
    sel.read_pixel(p[0].floor() as i32, p[1].floor() as i32, &mut v);
    v[0] > 0.0
}

/// Does a press with `tool` at `p` drag the selection rather than draw a new one? `Some(true)`
/// moves the floating piece (⌘ cuts one first, ⌘⌥ copies; a floating piece moves with a plain
/// drag; the Polygonal Lasso and Magic Wand need ⌘, as a plain press there places or samples),
/// `Some(false)` just the outline (no ⇧ / ⌥ and the options bar on New Selection, so a combining
/// drag still draws).
pub fn selection_drag_kind(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> Option<bool> {
    // Click-driven selection tools (a press places a point or samples): only ⌘ drags the
    // selection, and not while a polygon is being drawn.
    let clicky = matches!(tool, Tool::PolygonLasso | Tool::MagicWand);
    if !(clicky || matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee | Tool::Lasso)) || (clicky && !app.ui.polygon.is_empty()) {
        return None;
    }
    // ⌘ cuts, ⌘⌥ copies (`select.float`'s `copy`).
    let cut = mods.command && !mods.shift;
    if let Some(f) = app.session.active().and_then(photocraft_engine::float_cmds::floating) {
        // A press on the piece's outline (where its ants are) drags it; one in the margin the cut
        // left of the selection starts a new selection (#7538).
        let at = [p[0] - f64::from(f.offset.0), p[1] - f64::from(f.offset.1)];
        let on = f.parts.outline().map_or_else(|| inside_selection(app, at), |o| covers(o, at));
        return (on && !mods.shift && (!mods.alt || mods.command) && (cut || !clicky)).then_some(true);
    }
    if !inside_selection(app, p) {
        return None;
    }
    if cut {
        return Some(true);
    }
    (!clicky && !mods.command && selection_mode(app, mods) == "replace").then_some(false)
}

/// The tool a pointer event with `mods` goes to: ⌘ held is the Move tool (`hold_keys::cmd_moves`).
/// The canvas resolves the held key before the event (`tool_override`); automation and tests send
/// the modifier with the event.
pub(crate) fn event_tool(app: &PhotocraftApp, mods: egui::Modifiers) -> Tool {
    match app.active_tool() {
        t if app.tool_override.is_none() && mods.command && crate::hold_keys::cmd_moves(t) && app.ui.transform.is_none() && app.ui.text_edit.is_none() => {
            Tool::Move
        }
        t => t,
    }
}

/// Does a ⌘ (⌘⌥) press with selection tool `tool` at `p` move the whole layer (a duplicate with
/// ⌥), as the Move tool would? Outside the selection or without one; inside it, ⌘ drags the
/// selected pixels instead (`selection_drag_kind`).
pub(crate) fn command_moves_layer(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> bool {
    let selection_tool = matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee | Tool::Lasso | Tool::PolygonLasso | Tool::MagicWand);
    let floating = app.session.active().is_some_and(|st| photocraft_engine::float_cmds::floating(st).is_some());
    selection_tool && mods.command && !mods.shift && !floating && app.ui.polygon.is_empty() && !inside_selection(app, p)
}

/// The guide a ⌘ press with `tool` at `p` drags (vertical?, index), as the Move tool's press
/// would (#2690): ⌘ is the Move tool, and a guide under the pointer comes before the layer or the
/// selected pixels. The selection tools, and every tool on which ⌘ is the Move tool
/// (`hold_keys::cmd_moves`, #6288); the canvas shows the guide cursor where this finds a guide.
/// Not while a polygon or lasso outline, a stroke, Free Transform or type editing is under way.
pub(crate) fn command_guide_at(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> Option<(bool, usize)> {
    let selection_tool = matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee | Tool::Lasso | Tool::PolygonLasso | Tool::MagicWand);
    let moves = selection_tool || crate::hold_keys::cmd_moves(tool);
    let idle = app.ui.polygon.is_empty() && !crate::lasso_ui::active(app) && app.ui.transform.is_none() && app.ui.text_edit.is_none() && app.drag.is_none();
    (moves && mods.command && idle).then(|| crate::rulers::guide_at(app, p[0], p[1])).flatten()
}
