use super::tool_input::selection_drag_kind;
use super::*;

/// Photoshop's cursor over a selection: what a press (or the drag under way) would do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelCursor {
    /// A marquee or lasso inside the selection moves the outline (the move cursor, #1428), also
    /// while it is being dragged.
    Outline,
    /// Arrow with scissors: the Move tool (⌘ with a marquee) cuts and moves the selected pixels.
    Cut,
    /// Arrow with a second arrow: ⌥ lifts a copy of them instead.
    Copy,
    /// Plain arrow: a floating piece, moved again by a plain drag until it is dropped.
    Piece,
    /// Hollow arrowhead: the selected pixels are being dragged.
    Dragging,
}

/// The cursor at document point `p` with `tool` in effect and `mods` held (`None`: not over a
/// selection the press would move). The Move tool moves the selected pixels from anywhere
/// (`move_ui::moves_selected_pixels_with`); a selection tool from inside the selection, or on the
/// floating piece (`selection_drag_kind`).
pub fn selection_cursor(app: &PhotocraftApp, tool: Tool, p: [f64; 2], mods: egui::Modifiers) -> Option<SelCursor> {
    if let Some(d) = &app.drag {
        return match d.sel_move {
            Some(true) => Some(SelCursor::Dragging),
            Some(false) => Some(SelCursor::Outline),
            None => None,
        };
    }
    let floating = app.session.active().and_then(photocraft_engine::float_cmds::floating).is_some();
    let pixels = if tool == Tool::Move {
        crate::move_ui::moves_selected_pixels_with(app, tool)
    } else {
        match selection_drag_kind(app, tool, p, mods)? {
            false => return Some(SelCursor::Outline),
            true => true,
        }
    };
    if !pixels {
        return None;
    }
    // ⌥ (⌘⌥ with a selection tool) floats a copy, of the layer's pixels or of the piece.
    let copy = mods.alt && (tool == Tool::Move || mods.command);
    Some(match (floating, copy) {
        (_, true) => SelCursor::Copy,
        (true, false) => SelCursor::Piece,
        (false, false) => SelCursor::Cut,
    })
}

/// Whole-pixel offset of a selection drag in progress (`Drag::sel_move`).
pub(crate) fn selection_drag_delta(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let d = app.drag.as_ref().filter(|d| d.sel_move.is_some())?;
    let end = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    Some(((end[0] - d.start[0]).round().clamp(-1e7, 1e7) as i32, (end[1] - d.start[1]).round().clamp(-1e7, 1e7) as i32))
}

/// The selection a floating piece on document `idx` shows its ants for, before its offset: the
/// moved pixels' outline, as in Photoshop (#7538); `None` outlines the document's selection. The
/// surface's tiles are shared, so the clone is cheap.
pub(crate) fn floating_outline(app: &PhotocraftApp, idx: usize) -> Option<photocraft_raster::Surface> {
    if app.session.active_index() != Some(idx) {
        return None;
    }
    photocraft_engine::float_cmds::floating(app.session.active()?)?.parts.outline().cloned()
}

/// Where the selection is drawn relative to where it is: a floating piece's offset plus a drag
/// in progress.
pub(super) fn selection_shown_offset(app: &PhotocraftApp) -> Option<(i32, i32)> {
    crate::move_ui::floating_offset(app).or_else(|| selection_drag_delta(app)).or_else(|| carried_selection_delta(app))
}

/// Whole-pixel offset of a Move drag of several layers, which carries the selection along
/// (`move_ui::carries_selection`): their ants follow the layers while they move (#7538).
fn carried_selection_delta(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let d = app.drag.as_ref().filter(|d| d.tool == Tool::Move && d.sel_move.is_none())?;
    if !crate::move_ui::carries_selection(app) {
        return None;
    }
    let end = d.points.last().map_or(d.start, |p| [p[0], p[1]]);
    Some(((end[0] - d.start[0]).round().clamp(-1e7, 1e7) as i32, (end[1] - d.start[1]).round().clamp(-1e7, 1e7) as i32))
}

/// End of a selection drag: move the outline, or the floating piece. A click without moving
/// deselects, like a marquee click (a click on a floating piece leaves it floating, unless it never
/// moved: then it is put back, so Undo isn't spent on it).
pub(super) fn finish_selection_drag(app: &mut PhotocraftApp, floating: bool, start: [f64; 2], end: [f64; 2]) {
    let (dx, dy) = ((end[0] - start[0]).round(), (end[1] - start[1]).round());
    let unmoved = app.session.active().and_then(photocraft_engine::float_cmds::floating).is_some_and(|f| f.offset == (0, 0));
    let r = match (floating, dx == 0.0 && dy == 0.0) {
        (true, true) if unmoved => app.run("select.drop", json!({})),
        (true, true) => return,
        (false, true) if app.session.is_enabled("select.deselect") => app.run("select.deselect", json!({})),
        (false, true) => return,
        (true, false) => app.run("select.float", json!({"dx": dx, "dy": dy})),
        (false, false) => app.run("select.transformSelection", json!({"dx": dx, "dy": dy})),
    };
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}
