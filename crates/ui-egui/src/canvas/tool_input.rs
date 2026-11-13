use super::cancel_stale_vector_drag;
use super::cursors::{alt_eyedropper, sample_eyedropper};
use super::drag::Drag;
use super::gesture::{finish_release, polygon_click, selection_mode};
use super::selection::finish_selection_drag;
use super::strokes::{HOLD_REPEAT_MS, LiveStroke, begin_live_stroke, feed_live_stroke, paint_target, snap_aliased_tip, stroke_params, strokes_live};
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
            && app.live_stroke.as_ref().is_some_and(LiveStroke::wants_time);
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
    // A tool key pressed mid-drag ends the crop gesture here: a later handler can take every event
    // of the new tool, and a drag left set keeps the frame from committing on leave. It ends as a
    // release would, so a box drawn over the untouched default frame is committed, not dropped (#9223).
    if app.ui.tool != Tool::Crop {
        crate::crop_ui::end_drag(app);
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
            // Load Solid Colors Only loads the one colour under the hotspot (#11952).
            let (id, p) = if app.session.tools.brush.mixer.solid_colors_only {
                ("paint.mixerLoad", json!({"x": x, "y": y}))
            } else {
                ("brush.mixer.sample", json!({"points":[[x,y,pressure,pen.tilt_x,pen.tilt_y,pen.rotation]],"target":paint_target(app)}))
            };
            if let Err(e) = app.run(id, p) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
        return None;
    }
    Some((armed, mods))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ToolViewContext<'a> {
    pub(crate) xf: &'a super::ViewXform,
    pub(crate) viewport: Option<egui::ViewportId>,
}

impl<'a> ToolViewContext<'a> {
    /// Creates a view context pairing the current canvas transform with an optional viewport identity.
    pub(crate) fn new(xf: &'a super::ViewXform, viewport: Option<egui::ViewportId>) -> Self {
        Self { xf, viewport }
    }
}

/// The handlers a pointer event meets before the tool: dropping a floating piece the press won't
/// drag, snapping, and the overlays and modifier gestures (Free Transform, sticky keys, brush
/// resize, the ⌥ eyedropper, path and shape widgets…) that take the event for themselves. `None`:
/// one of them handled it; otherwise the event as it came and as filtered (snapped, the Move
/// tool's axis lock), and the modifiers with the sticky keys applied.
fn filter_event_before_tool(
    app: &mut PhotocraftApp,
    ev: ToolEvent,
    mods: egui::Modifiers,
    armed: bool,
    view: Option<ToolViewContext<'_>>,
) -> Option<(ToolEvent, ToolEvent, egui::Modifiers)> {
    // View › Snap / Snap To and smart guides (snap_ui.rs).
    let raw = ev;
    // The Move tool's marquee selecting layers takes its moves and release unsnapped (move_box.rs).
    if crate::move_box::pointer(app, raw) {
        return None;
    }
    // A press that doesn't drag the floating piece drops it first: off it, with ⇧ / ⌥ to draw, or a
    // plain selection-tool press on it, which moves the outline only (#11368). The Move tool (⌘
    // with a painting tool too, `event_tool`) drags it from anywhere.
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
        if matches!(transform_ev, ToolEvent::Up { .. }) && app.ui.frame_tool.auto_transform {
            app.ui.frame_tool.auto_transform = false;
            crate::transform_tool::commit(app);
        }
        return None;
    }
    if crate::distort_ui::pointer(app, ev, mods) {
        return None;
    }
    if crate::perspective_crop_ui::pointer(app, ev, mods) {
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
    // The box round several selected shape layers: its handles scale them (Path Selection and the
    // shape tools).
    if crate::shape_bounds_box::pointer(app, raw, mods) {
        return None;
    }
    // Direct Selection, and the Pen's ⌘ (Direct Selection) and ⌥ (Convert Point) modes (#790).
    let xf = view.map(|v| v.xf);
    let viewport = view.and_then(|v| v.viewport);
    if crate::type_path_placement::pointer_with_context(app, raw, xf, viewport) {
        return None;
    }
    // Pen + ⌥ on a straight segment of the active path adds an anchor and drags out its handles
    // (#12899); anywhere else ⌥ stays Convert Point, below.
    if let ToolEvent::Down { x, y, .. } = ev
        && crate::anchor_tools::pen_alt_segment_press(app, mods, [x, y])
    {
        return None;
    }
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

/// Remember a Move-way pick that changed the active layer (with the engine's change count after it, so
/// any later change invalidates the marker), so the Layers panel's
/// reveal can honour Expand layer group on click (layer_reveal.rs).
fn note_move_pick(app: &mut PhotocraftApp, active_before: Option<photocraft_doc::LayerId>) {
    let active_after = app.session.active().and_then(|st| st.active_layer);
    if active_after != active_before {
        app.move_pick = active_after.map(|id| (id, app.session.active_layer_changes()));
    }
}

/// Handle Move-specific pointer events and report whether the event was consumed. Auto-Select
/// picks on press so snapping and dragging use the resulting layer selection. When a locked-only
/// hit starts a marquee, Shift preserves the selection that the marquee adds to.
fn handle_move_pointer(app: &mut PhotocraftApp, raw: ToolEvent, ev: ToolEvent, tool: Tool, mods: egui::Modifiers) -> bool {
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
                app.ui.status = "Move and transforms on several color channels are not implemented yet".into();
                return true;
            }
            // Auto-Select (or ⌘-click while it is off) picks the layer under the pointer first
            // (not when the selected pixels move: those are the active layer's).
            if !crate::move_ui::moves_selected_pixels_with(app, tool) && app.ui.tool_options.move_auto_select != mods.command {
                let target = app.ui.tool_options.move_target.clone();
                // A held temporary Move tool keeps the ordinary point-pick behavior.
                let move_auto_select = app.ui.tool == Tool::Move;
                let mode = if !mods.shift {
                    "replace"
                } else if move_auto_select {
                    "toggle"
                } else {
                    "add"
                };
                let probe = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "select": false, "moveAutoSelect": move_auto_select}));
                if probe.as_ref().is_ok_and(|v| v["layer"].is_null()) {
                    let empty = crate::move_box::empty_at(app, [x, y]);
                    if empty || move_auto_select {
                        if !empty {
                            // Pressing locked selected pixels still uses the existing move-lock
                            // handler. Otherwise a locked-only hit can start a marquee (#4058).
                            if inside_selection(app, [x, y]) && crate::move_lock::blocked(app, tool, [x, y], mods) {
                                app.move_blocked = true;
                                return true;
                            }
                            // Shift preserves the base selection for its additive marquee.
                            if !mods.shift {
                                let _ = app.run("select.deselectLayers", json!({}));
                            }
                        }
                        crate::move_box::begin(app, [x, y], mods.shift, move_auto_select, move_auto_select && !empty);
                        return true;
                    }
                } else {
                    let locked_hit = !move_auto_select
                        && probe.as_ref().ok().and_then(|v| v["layer"].as_u64()).is_some_and(|id| {
                            app.session.active().is_some_and(|st| {
                                let id = photocraft_doc::LayerId(id);
                                let locks = st.doc.effective_locks(id);
                                locks.position || locks.all || st.doc.is_background(id)
                            })
                        });
                    let before = app.session.active().map(|st| st.selected_layers());
                    let active_before = app.session.active().and_then(|st| st.active_layer);
                    let _ = app.run("layer.pickAt", json!({"x": x, "y": y, "target": target, "mode": mode, "moveAutoSelect": move_auto_select}));
                    note_move_pick(app, active_before);
                    if locked_hit {
                        crate::move_box::begin(app, [x, y], mods.shift, false, false);
                        return true;
                    }
                    // Snapping (and the drag's box) started before the pick: point it at what moves.
                    if app.session.active().map(|st| st.selected_layers()) != before {
                        crate::snap_ui::retarget_move(app, [x, y], mods);
                    }
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
                let before = app.session.active().and_then(photocraft_engine::float_cmds::floating).cloned();
                if crate::move_ui::float_selected_for_drag(app, mods.alt) {
                    let mut d = Drag::new(tool, [x, y], vec![[x, y, pressure as f64]], mods, false);
                    d.sel_move = Some(true);
                    d.transfer_floating_before = before;
                    app.drag = Some(d);
                }
                return true;
            }
        }
        ToolEvent::Move { x, y, .. } => {
            if let Some(mut d) = app.guide_drag.filter(|d| d.index.is_some()) {
                d.follow_pointer(app, unsnapped(raw), [x, y], mods);
                app.guide_drag = Some(d);
                return true;
            }
        }
        ToolEvent::Up { x, y } => {
            if let Some(mut d) = app.guide_drag.filter(|d| d.index.is_some()) {
                app.guide_drag = None;
                d.follow_pointer(app, unsnapped(raw), [x, y], mods);
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

/// A Mixer Brush press the engine refused: Indexed Color and Bitmap documents answer it with the
/// reference's alert (#11958); anything else goes to the status bar, as before.
fn mixer_press_failed(app: &mut PhotocraftApp, message: String) {
    use photocraft_engine::brush_cmds::mixer_live::{BITMAP_REFUSAL, INDEXED_REFUSAL};
    if message == INDEXED_REFUSAL || message == BITMAP_REFUSAL {
        // One alert, even if more presses arrive while it is open (`ui.pointer` can send them).
        let shown =
            app.ui.dialogs.iter().any(|d| d.kind == crate::state::DialogKind::Error && d.fields.get("message").and_then(|m| m.as_str()) == Some(&message));
        if !shown {
            let params = json!({ "message": message }).as_object().cloned().unwrap_or_default();
            app.ui.open_dialog(crate::state::DialogKind::Error, params);
        }
    } else {
        app.ui.status = message;
        app.ui.status_error = true;
    }
}

/// A Red Eye click. CMYK, Grayscale and Indexed Color documents get the reference's alert
/// (#12575), once even if more clicks arrive while it is open (`ui.pointer` can send them).
fn red_eye_click(app: &mut PhotocraftApp, x: f64, y: f64) {
    use photocraft_engine::redeye_cmds::MODE_REFUSAL;
    let o = app.ui.tool_options.clone();
    let r = app.run(
        "paint.redEye",
        json!({
            "x": x.floor(),
            "y": y.floor(),
            "pupilSize": o.red_eye_pupil_size.clamp(1.0, 100.0),
            "darken": o.red_eye_darken.clamp(0.0, 100.0),
            "target": paint_target(app),
        }),
    );
    if r.is_err_and(|e| e == MODE_REFUSAL) {
        app.ui.status.clear();
        app.ui.status_error = false;
        let message = tl!(MODE_REFUSAL);
        let kind = crate::state::DialogKind::Error;
        if !app.ui.dialogs.iter().any(|d| d.kind == kind && d.fields.get("message").and_then(|m| m.as_str()) == Some(message)) {
            app.ui.open_dialog(kind, json!({ "message": message }).as_object().cloned().unwrap_or_default());
        }
    }
}

/// A Paint Bucket click. A Bitmap document gets the reference's alert (#12536), once even if
/// more clicks arrive while it is open (`ui.pointer` can send them).
fn bucket_click(app: &mut PhotocraftApp, x: f64, y: f64) {
    use photocraft_engine::paint_cmds::BITMAP_REFUSAL;
    let o = app.ui.tool_options.clone();
    let contents = if o.bucket_fill_pattern { "pattern" } else { "foreground" };
    let mode = if o.bucket_clear { "clear" } else { o.bucket_blend_mode.label() };
    let r = app.run("paint.bucket", json!({"x": x.floor(), "y": y.floor(), "tolerance": o.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "sampleAllLayers": o.sample_all.paint_bucket, "opacity": o.fill_opacity, "contents": contents, "mode": mode, "target": paint_target(app)}));
    if r.is_err_and(|e| e == BITMAP_REFUSAL) {
        app.ui.status.clear();
        app.ui.status_error = false;
        let message = tl!(BITMAP_REFUSAL);
        let kind = crate::state::DialogKind::Error;
        if !app.ui.dialogs.iter().any(|d| d.kind == kind && d.fields.get("message").and_then(|m| m.as_str()) == Some(message)) {
            app.ui.open_dialog(kind, json!({ "message": message }).as_object().cloned().unwrap_or_default());
        }
    }
}

/// The Brush's chosen Clear paints as Normal on the Background, and its Mode then reads Normal
/// (#11897): at the press of a Brush stroke on the Background's own pixels, Clear (chosen in the
/// options bar, not a held `~`) is turned off, so the stroke is sent as Normal. A Background whose
/// pixels are locked refuses the stroke, so Clear stays chosen there.
fn clear_off_on_background(app: &mut PhotocraftApp, tool: Tool) {
    let on_background = app.session.active().is_some_and(|st| {
        st.active_layer.is_some_and(|id| {
            st.doc.is_background(id) && {
                let locks = st.doc.effective_locks(id);
                !locks.pixels && !locks.all
            }
        })
    });
    if tool == Tool::Brush && app.ui.tool_options.brush_clear && !app.held_erase && on_background && paint_target(app) == serde_json::json!("pixels") {
        app.ui.tool_options.brush_clear = false;
    }
}

/// Tool state machine. Shared by mouse input and automation.
pub fn tool_event(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers) {
    tool_event_with_view(app, ev, mods, None);
}

/// Dispatches a tool event through preflight, pre-tool overlays, and tool handlers within an explicit view context.
pub(super) fn tool_event_with_view(app: &mut PhotocraftApp, ev: ToolEvent, mods: egui::Modifiers, view: Option<ToolViewContext<'_>>) {
    if consumes_cancelled_vector_event(app, ev) {
        return;
    }
    let Some((armed, mods)) = preflight_tool_event(app, ev, mods) else { return };
    let Some((raw, ev, mods)) = filter_event_before_tool(app, ev, mods, armed, view) else { return };
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
    if handle_move_pointer(app, raw, ev, tool, mods) {
        return;
    }

    match ev {
        ToolEvent::Down { x, y, pressure } => {
            // Painting a type, shape, Smart Object or fill layer asks to rasterize it first
            // (⌥-click with the Clone Stamp or Healing Brush only sets the source).
            let sets_source = matches!(tool, Tool::CloneStamp | Tool::Healing) && mods.alt;
            // Painting on or moving a hidden layer is refused at the press, as in Photoshop (#571).
            if !sets_source && press_refused(app, tool) {
                return;
            }
            // The Clone Stamp's no-source alert opens at the press, after any rasterize prompt.
            if !sets_source
                && (crate::rasterize_prompt::intercept(app, tool, x, y, pressure)
                    || refuse_group(app, tool)
                    || crate::retouch_ui::clone_stamp_without_source(app, tool))
            {
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
            // pixels into a floating piece (`select.float`) and drag that. Inside is judged where the
            // press was before snapping, as `snap_ui::begin` judges it: snapped, a press just outside
            // can land on the selection's edge (#11392).
            if let Some(cut) = selection_drag_kind(app, tool, unsnapped(raw), mods) {
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
                        crate::path_components::untarget_saved_path_off(app, [x, y], tol);
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
                    if wand_pasteboard_click(app, x, y, mode) {
                        return;
                    }
                    let _ = app.run("select.magicWand", json!({"x": x.floor(), "y": y.floor(), "tolerance": o.tolerance, "contiguous": o.contiguous, "antiAlias": o.anti_alias, "sampleAllLayers": o.sample_all.magic_wand, "mode": mode}));
                    return;
                }
                Tool::MagicEraser => {
                    crate::eraser_ui::click(app, tool, x, y);
                    return;
                }
                Tool::PaintBucket => {
                    bucket_click(app, x, y);
                    return;
                }
                Tool::RedEye => {
                    red_eye_click(app, x, y);
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
            clear_off_on_background(app, tool);
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
            // Erase to History (or ⌥ for this stroke): the Eraser's brush stroke paints the History
            // panel's source back.
            let tool = crate::eraser_ui::drag_tool(app, tool, mods.alt);
            let mut drag = Drag { clear, line: from.is_some(), ..Drag::new(tool, from.map_or([x, y], |p| [p[0], p[1]]), points, mods, erase) };
            drag.capture_vector_owners(app);
            drag.history_params = crate::retouch_ui::history_params_at_press(app, tool, mods.alt);
            drag.capture_selection_at_press(app);
            drag.document = super::drag::DragDocument::capture(app);
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
                        mixer_press_failed(app, e.to_string());
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
            note_left_press(app, raw);
            tool_move(app, x, y, pressure, mods);
            feed_live_stroke(app);
        }
        ToolEvent::Up { x, y } => {
            // Before `resize` is cleared: a background box drag applies its box now.
            crate::type_box_preview::flush(app);
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
            let (click, [x, y]) = selection_release(&d, raw, [x, y]);
            // A Move-tool click (released where it was pressed) selects, it never moves: snapping
            // the release point would otherwise nudge the layer onto a nearby edge. A drag that
            // snapping held at the press point (no points recorded) and that came back there is
            // not a click: it keeps a floating piece floating (#11392).
            if d.tool == Tool::Move && !d.left_press && d.points.len() < 2 && matches!(raw, ToolEvent::Up { x, y } if [x, y] == d.start) {
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
            finish_release(app, d, click);
            crate::move_mods::finish(app);
        }
    }
}

/// Where pointer event `raw` (before snapping) was.
fn unsnapped(raw: ToolEvent) -> [f64; 2] {
    match raw {
        ToolEvent::Down { x, y, .. } | ToolEvent::Move { x, y, .. } | ToolEvent::Up { x, y } => [x, y],
    }
}

/// Is `p` within half a pixel of the press of `d` (a click's reach, a zoomed-in hand's jitter
/// included)?
fn at_press(d: &Drag, p: [f64; 2]) -> bool {
    (p[0] - d.start[0]).round() == 0.0 && (p[1] - d.start[1]).round() == 0.0
}

/// Records on the drag that pointer move `raw` (before snapping) left the press (`Drag::left_press`).
fn note_left_press(app: &mut PhotocraftApp, raw: ToolEvent) {
    if let ToolEvent::Move { x, y, .. } = raw
        && let Some(d) = app.drag.as_mut()
    {
        d.left_press |= !at_press(d, [x, y]);
    }
}

/// Whether the release `raw` of selection drag `d` is a click, and where it ends (`p`: snapped). A
/// click never leaves the press (`at_press`) before snapping, and ends where it was pressed, as
/// with snapping off: snapping the release would nudge the selection's edges onto a nearby guide or
/// the canvas edge, moving it instead of deselecting (a Single Row / Column Marquee: selecting the
/// line). A drag that snapping pulls back onto the press point, or that comes back to it, is not a
/// click (#11392).
fn selection_release(d: &Drag, raw: ToolEvent, p: [f64; 2]) -> (bool, [f64; 2]) {
    match raw {
        ToolEvent::Up { x, y } if d.sel_move.is_some() && !d.left_press && at_press(d, [x, y]) => (true, d.start),
        _ => (false, p),
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

/// The press-time refusals: a hidden target layer (#571), and for the tools with an alert in
/// `brush_refusal` (the Brush, #11898, and others) a locked layer or a group's pixels. Those tools
/// say why in an alert, as in Photoshop; the other tools say a hidden target in the status bar
/// (the Remove tool mostly says nothing: [`quiet_hidden_refusal`]). True when the press is refused.
fn press_refused(app: &mut PhotocraftApp, tool: Tool) -> bool {
    let hidden = hidden_target(app, tool);
    if let Some(message) = crate::brush_refusal::alert(app, tool, hidden.is_some()) {
        crate::brush_refusal::prompt(app, &message);
        return true;
    }
    if let Some(why) = hidden {
        if !quiet_hidden_refusal(app, tool) {
            app.ui.status = why.into();
            app.ui.status_error = true;
        }
        return true;
    }
    false
}

/// A press with `tool` that would paint the pixels of a group (the active layer, with neither its
/// mask, an alpha channel nor the Quick Mask targeted): a group has none, so refuse it with
/// Photoshop's message instead of drawing a stroke that vanishes on release with only a
/// status-bar note (#10074). Returns whether it was refused.
fn refuse_group(app: &mut PhotocraftApp, tool: Tool) -> bool {
    let group = crate::rasterize_prompt::paints_pixels(app, tool)
        && paint_target(app) == json!("pixels")
        && app.session.active().and_then(|st| st.doc.layer(st.active_layer?)).is_some_and(photocraft_doc::Layer::is_group);
    if group {
        let message = crate::i18n::fmt(tl!("Could not use the {tool} because the target layer is a group."), &[("tool", tl!(tool.label()))]);
        app.ui.open_dialog(crate::state::DialogKind::Error, json!({"message": message}).as_object().cloned().unwrap_or_default());
    }
    group
}

/// A Remove press on a hidden active pixel layer does nothing and says nothing, as in Photoshop
/// (#12554). Only that case was measured: a layer inside a hidden group, a targeted mask, Sample
/// All Layers and the other layer kinds keep the status-bar error. Hidden is checked before the
/// lock, so a hidden layer with Lock All is quiet too.
fn quiet_hidden_refusal(app: &PhotocraftApp, tool: Tool) -> bool {
    if tool != Tool::Remove || app.ui.tool_options.sample_all.remove || paint_target(app) != json!("pixels") {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let Some(id) = st.active_layer.filter(|&id| st.doc.layer(id).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Raster(_)))) else {
        return false;
    };
    let Some(path) = st.doc.path_of(id) else { return false };
    (1..path.len()).all(|n| path.get(..n).and_then(|p| st.doc.layer_at(p)).is_some_and(|l| l.visible))
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

/// A New-mode Magic Wand click on the pasteboard deselects, as Photoshop's step "Deselect"
/// (#11782), instead of a "Magic Wand" step that selects nothing. True when it handled the click.
fn wand_pasteboard_click(app: &mut PhotocraftApp, x: f64, y: f64, mode: &str) -> bool {
    let on_canvas = app.session.active().is_some_and(|st| st.doc.bounds().contains(x.floor() as i32, y.floor() as i32));
    if mode != "replace" || on_canvas {
        return false;
    }
    if app.session.is_enabled("select.deselect") {
        let _ = app.run("select.deselect", json!({}));
    }
    true
}

/// Does a press with `tool` at `p` drag the selection rather than draw a new one? `Some(true)`
/// moves the floating piece (⌘ cuts one first, ⌘⌥ copies; the Polygonal Lasso and Magic Wand need
/// ⌘, as a plain press there places or samples), `Some(false)` just the outline (no ⇧ / ⌥ and the
/// options bar on New Selection, so a combining drag still draws). A plain drag on a floating piece
/// moves the outline too, as in Photoshop: the press drops the piece first (#11368). The Single Row
/// / Single Column Marquee drag the outline too (#11392); a click without moving, inside the
/// selection or on a floating piece (dropped first), still selects the line under the pointer
/// (`finish_gesture`).
pub fn selection_drag_kind(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> Option<bool> {
    // Click-driven selection tools (a press places a point or samples): only ⌘ drags the
    // selection, and not while a polygon is being drawn.
    let clicky = matches!(tool, Tool::PolygonLasso | Tool::MagicWand);
    // Object Selection with Object Finder on drags the outline like a marquee in New mode (#12853).
    // With Object Finder off a press stays a Quick Selection dab or a rectangle, as before (the
    // issue measured only Finder on). Unlike the marquees, ⌘ with it is the Move tool
    // (`hold_keys::cmd_moves`), which moves the pixels itself: a ⌘ press must not start a cut here
    // first.
    let marquee = matches!(tool, Tool::RectMarquee | Tool::EllipseMarquee | Tool::SingleRowMarquee | Tool::SingleColumnMarquee)
        || (tool == Tool::ObjectSelection && app.ui.tool_options.object_finder);
    if tool == Tool::ObjectSelection && mods.command {
        return None;
    }
    // In Indexed Color and Bitmap the press must reach `finish_object_selection` for the tool's
    // alert (#12867) rather than deselect or move the outline.
    if tool == Tool::ObjectSelection && app.session.active().is_some_and(|st| photocraft_engine::smartselect_cmds::object_mode_refusal(st.doc.mode).is_some()) {
        return None;
    }
    if !(clicky || marquee || tool == Tool::Lasso) || (clicky && !app.ui.polygon.is_empty()) {
        return None;
    }
    // ⌘ cuts, ⌘⌥ copies (`select.float`'s `copy`).
    let cut = mods.command && !mods.shift;
    if let Some(f) = app.session.active().and_then(photocraft_engine::float_cmds::floating) {
        // A ⌘ press on the piece's outline (where its ants are) drags it; a plain one drags the
        // outline, dropping the piece where it is first (`filter_event_before_tool`, #11368); one
        // in the margin the cut left of the selection starts a new selection (#7538).
        let at = [p[0] - f64::from(f.offset.0), p[1] - f64::from(f.offset.1)];
        let on = f.parts.outline().map_or_else(|| inside_selection(app, at), |o| covers(o, at));
        if !on {
            return None;
        }
        if cut {
            return Some(true);
        }
        return (!clicky && !mods.command && selection_mode(app, mods) == "replace").then_some(false);
    }
    if !inside_selection(app, p) {
        return None;
    }
    if cut {
        return Some(true);
    }
    // The Magic Wand in New mode drags the outline like a marquee: a click deselects, a drag
    // moves the outline (#11782).
    let outline = !clicky || tool == Tool::MagicWand;
    (outline && !mods.command && selection_mode(app, mods) == "replace").then_some(false)
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

#[cfg(test)]
#[path = "art_history_shift_click_tests.rs"]
mod art_history_shift_click_tests;
