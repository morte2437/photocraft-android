use super::history::history;
use super::info::info_panel;
use super::layers::{channels, layers};
use super::navigator::navigator;
use super::properties::{adjustments_grid, properties_body};
use super::{PhotocraftApp, Stroke, Tokens, icons, pos2};

// ----------------------------------------------------------------------------- dock

/// The panels and the canvas in the space left by the menus, bars and toolbar: craft-ui's
/// docking layout, with the icon rail at the window's end edge in the Studio themes.
pub fn dock_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    if app.ui.panels.rail && !t.pro {
        rail(app, ui, &t);
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::panel_docking::show(app, ui));
    crate::dock::persist(app, ui.ctx());
    crate::color_panel_ui::persist(app);
}

/// The dock and its icon rail as one right panel, for the layout with the toolbox between the
/// canvas and the dock. The canvas is drawn separately afterwards. Keeping the rail inside this
/// panel means the toolbox cannot cover it when it is one column wide.
pub fn dock_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // Lets the frame tick expire a drag start if this panel stops rendering (dock hidden).
    let frame = ui.ctx().cumulative_frame_nr();
    ui.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("dock-edge-rendered-frame"), frame));
    let t = Tokens::get(ui.ctx());
    let (rail_w, _) = rail_widths(t.pro);
    let rail_shown = app.ui.panels.rail;
    let rw = if rail_shown { rail_w } else { 0.0 };
    let content = ui.available_rect_before_wrap();
    let range = crate::panels::dock_width_range(content.width());
    // The dock must leave the toolbox and the canvas's 200-point minimum room: the standard
    // layout enforces the canvas minimum inside craft-ui, the fixed-size dock-edge panel does not.
    let toolbar_w = super::toolbar::toolbar_width(t.pro, app.ui.panels.toolbar_double);
    let max_dock = (*range.end()).min((content.width() - toolbar_w - crate::panel_docking::CANVAS_MIN_WIDTH - rw).max(1.0));
    let min_dock = (*range.start()).min(max_dock);
    let locked = app.session.prefs().workspace_locked;
    let modal = crate::panel_docking::modal_open(app, ui.ctx());
    crate::panel_docking::apply_pending_dock_width(app, ui.ctx());
    let metrics = crate::panel_docking::dock_metrics(app);
    let empty = metrics.empty;
    if empty {
        // Nothing to resize or draw in the dock column, but Studio draws its floating Properties
        // group from the docking pass: keep that pass, on the full remaining area, with no column.
        if rail_shown {
            crate::direction::end_panel("dock-column").resizable(false).show_separator_line(false).exact_size(rw).frame(egui::Frame::NONE.fill(t.chrome)).show(
                ui,
                |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let rect = ui.max_rect();
                    let inner = rect.shrink2(egui::vec2(4.0, 8.0));
                    let mut rail_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                    rail_body(app, &mut rail_ui, &t);
                },
            );
        }
        crate::panels::clear_dock_edge_drag(ui.ctx());
        // A child that does not consume the parent's layout space: the docking pass still draws
        // floating windows, but the toolbox and canvas keep their room.
        let mut dock_ui = ui.new_child(egui::UiBuilder::new().max_rect(ui.max_rect()));
        crate::panel_docking::show_dock(app, &mut dock_ui);
        crate::dock::persist(app, ui.ctx());
        crate::color_panel_ui::persist(app);
        return;
    }
    let iconic = metrics.iconic_only;
    let natural_min = if iconic { metrics.minimum } else { metrics.minimum.max(min_dock) };
    let max_dock = max_dock.min(if metrics.maximum.is_finite() { metrics.maximum.max(natural_min) } else { max_dock });
    let feasible = natural_min <= max_dock + 0.5;
    let min_visible = natural_min.min(max_dock);
    // Draw the width the layout actually has, not the minimum the gestures enforce.
    let mut stored = metrics.width.clamp(1.0, max_dock);
    let generation = app.ui.docking_generation;
    let drag_id = egui::Id::unique("dock-edge-drag");
    // Resizing is unavailable while locked, modal, when the parent UI is disabled (screen picking
    // disables it), or when the window cannot fit the column minima. A disabled handle never
    // receives drag_stopped, so drop any stored start here; a start from another workspace
    // generation is stale and must not touch this one.
    let resizable = !locked && !modal && ui.is_enabled() && feasible;
    let mut drag = match if resizable { ui.ctx().data(|d| d.get_temp::<(f32, f32, u64)>(drag_id)) } else { None } {
        Some((start_x, start_w, saved)) if saved == generation => Some((start_x, start_w)),
        Some(_) => {
            crate::panels::clear_dock_edge_drag(ui.ctx());
            None
        }
        None => {
            if !resizable {
                crate::panels::clear_dock_edge_drag(ui.ctx());
            }
            None
        }
    };
    let lower = (min_visible + rw).min(max_dock + rw);
    let upper = max_dock + rw;
    // A gesture applies its minimum even without motion; idle may retain a narrower saved width.
    let pointer_width = |start_x: f32, start_w: f32| {
        let pointer = ui.ctx().input(|i| i.pointer.latest_pos()).map_or(start_x, |pos| pos.x);
        let dx = if crate::direction::is_rtl() { pointer - start_x } else { start_x - pointer };
        (start_w + dx).clamp(lower, upper)
    };
    // The release frame still carries the final pointer position; apply it before dropping the
    // drag, so the dock keeps the width the user released at instead of the previous frame's.
    if let Some((start_x, start_w)) = drag
        && !ui.ctx().input(|i| i.pointer.any_down())
    {
        let final_w = pointer_width(start_x, start_w);
        let requested = (final_w - rw).clamp(min_visible, max_dock);
        stored = crate::panel_docking::set_end_dock_total(app, requested).unwrap_or(requested).clamp(1.0, max_dock);
        drag = None;
        ui.ctx().data_mut(|d| d.remove::<(f32, f32, u64)>(drag_id));
    }
    let mut panel_w = if let Some((start_x, start_w)) = drag { pointer_width(start_x, start_w) } else { stored + rw };
    if drag.is_some() && resizable {
        let requested = (panel_w - rw).clamp(min_visible, max_dock);
        if let Some(actual) = crate::panel_docking::set_end_dock_total(app, requested) {
            // The model can land somewhere other than the pointer request (minimums, iconic
            // caps); draw the panel at the width that was actually stored.
            panel_w = (actual + rw).clamp(lower, upper);
        }
    }
    let panel =
        crate::direction::end_panel("dock-column").resizable(false).show_separator_line(false).exact_size(panel_w).frame(egui::Frame::NONE.fill(t.card)).show(
            ui,
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let height = ui.available_height();
                let mut dock_rect = ui.max_rect();
                if rail_shown {
                    // The rail sits between the dock (at the window edge) and the toolbox, with the
                    // chrome fill and inset the standalone rail panel gives it.
                    let rail_rect = if crate::direction::is_rtl() {
                        egui::Rect::from_min_size(egui::pos2(dock_rect.right() - rw, dock_rect.top()), egui::vec2(rw, height))
                    } else {
                        egui::Rect::from_min_size(dock_rect.min, egui::vec2(rw, height))
                    };
                    ui.painter().rect_filled(rail_rect, 0.0, t.chrome);
                    ui.allocate_rect(rail_rect, egui::Sense::hover());
                    let inner = rail_rect.shrink2(egui::vec2(4.0, 8.0));
                    let mut rail_ui = ui.new_child(egui::UiBuilder::new().max_rect(inner));
                    rail_body(app, &mut rail_ui, &t);
                    if crate::direction::is_rtl() {
                        dock_rect.max.x = rail_rect.left();
                    } else {
                        dock_rect.min.x = rail_rect.right();
                    }
                }
                let mut dock_ui = ui.new_child(egui::UiBuilder::new().max_rect(dock_rect));
                // The docking pass also renders floating windows (Studio floats Properties over the
                // canvas), so it runs even when the dock column itself is empty.
                crate::panel_docking::show_dock(app, &mut dock_ui);
                // Block the docking layout's column-edge handle at the dock's content edge. It is
                // registered before the resize handle so the handle wins where they overlap when the
                // rail is hidden and both edges coincide; one point either side keeps tab strips and
                // scrollbars clear.
                let dock_edge = if crate::direction::is_rtl() { dock_rect.right() } else { dock_rect.left() };
                let cover = egui::Rect::from_min_max(egui::pos2(dock_edge - 1.0, dock_rect.top()), egui::pos2(dock_edge + 1.0, dock_rect.bottom()));
                let _ = ui.interact(cover, egui::Id::unique("dock-content-edge"), egui::Sense::click_and_drag());
                // The panel's inner edge (between the toolbox and the rail) is the resize handle.
                let panel_rect = ui.max_rect();
                let panel_edge = if crate::direction::is_rtl() { panel_rect.right() } else { panel_rect.left() };
                let handle = egui::Rect::from_min_max(egui::pos2(panel_edge - 4.0, panel_rect.top()), egui::pos2(panel_edge + 4.0, panel_rect.bottom()));
                // A locked workspace, an empty dock (nothing to resize), a modal dialog or a disabled
                // UI leaves the handle inert.
                let sense = if resizable { egui::Sense::drag() } else { egui::Sense::hover() };
                let response = ui.interact(handle, egui::Id::unique("dock-edge-handle"), sense);
                // The bar is always visible, on the foreground layer so no panel, rail or dock
                // content can hide it; it brightens under the pointer.
                let active = resizable && (response.hovered() || response.dragged());
                if active {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
                let line = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("dock-edge-line"));
                // The theme's separator colour at rest, the accent while the edge is hovered or
                // dragged: a dark divider that lights up, not a permanently lit bar.
                let color = if active { t.accent } else { t.separator };
                ui.ctx().layer_painter(line).vline(panel_edge, panel_rect.y_range(), Stroke::new(1.0, color));
                if response.drag_started() {
                    let start_x = response.interact_pointer_pos().map_or(panel_edge, |pos| pos.x);
                    ui.ctx().data_mut(|d| d.insert_temp(drag_id, (start_x, panel_w, generation)));
                }
                if response.dragged() {
                    ui.ctx().request_repaint();
                }
                if response.drag_stopped() {
                    ui.ctx().data_mut(|d| d.remove::<(f32, f32, u64)>(drag_id));
                }
            },
        );
    crate::panel_docking::record::dock_panel_rect(ui.ctx(), panel.response.rect);
    crate::dock::persist(app, ui.ctx());
    crate::color_panel_ui::persist(app);
}

/// True while the dock-edge resize drag has a stored start: a synthetic/accessibility click can
/// arrive with no pointer button down, but the in-progress dock width must not be persisted.
pub(crate) fn dock_edge_drag_active(ctx: &egui::Context) -> bool {
    ctx.data(|data| data.get_temp::<(f32, f32, u64)>(egui::Id::unique("dock-edge-drag")).is_some())
}

/// Drops a dock-edge drag start that can no longer be completed (its panel stopped rendering).
pub(crate) fn clear_dock_edge_drag(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<(f32, f32, u64)>(egui::Id::unique("dock-edge-drag")));
}

/// The width the dock column took in the last frame (tests: an iconic-only dock must take its
/// narrow width and return the rest to the canvas).
#[cfg(test)]
pub(crate) fn dock_panel_width(ctx: &egui::Context) -> f32 {
    dock_panel_rect(ctx).width()
}

/// The actual allocated dock panel rectangle, not its requested pre-render width (tests).
#[cfg(test)]
pub(crate) fn dock_panel_rect(ctx: &egui::Context) -> egui::Rect {
    ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("photo-dock-panel-rect")).unwrap_or(egui::Rect::NOTHING))
}

/// The docking layout's protected canvas panel: the document area, with the icon rail on its end
/// side, beside the dock, in the Pro themes.
pub(crate) fn canvas_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let backdrop = if app.ui.view.hides_chrome() { egui::Color32::BLACK } else { crate::prefs_ui::pasteboard_color(app).unwrap_or(t.canvas) };
    if app.ui.panels.rail && t.pro {
        rail(app, ui, &t);
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(backdrop)).show(ui, |ui| crate::canvas::document_area(app, ui));
}

/// The icon rail's width and its buttons' size, in points.
fn rail_widths(pro: bool) -> (f32, f32) {
    if pro { (36.0, 28.0) } else { (44.0, 32.0) }
}

/// Narrow icon rail, unless an embedding app hides it: shows, expands or collapses panel groups.
fn rail(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let (rw, _) = rail_widths(t.pro);
    crate::direction::end_panel("rail")
        .resizable(false)
        .exact_size(rw)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(4, 8)))
        .show(ui, |ui| rail_body(app, ui, t));
}

/// The rail's buttons, without the panel that normally holds them.
fn rail_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let customized = app.ui.docking.is_some();
    let p = app.ui.panels.clone();
    let (_, rb) = rail_widths(t.pro);
    // The separator is on the canvas side, inside the rail's inset.
    let r = ui.max_rect();
    let x = if crate::direction::is_rtl() { r.right() } else { r.left() };
    ui.painter().line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(1.0, t.separator));
    ui.spacing_mut().item_spacing.y = 4.0;
    use crate::dock::Group;
    let entries: [(&str, &str, Group); 6] = [
        ("sliders-horizontal", tl!("Properties"), Group::Properties),
        ("navigation", tl!("Navigator"), Group::Navigator),
        ("palette", tl!("Color & Swatches"), Group::Color),
        ("layers", tl!("Layers"), Group::Layers),
        ("clock", tl!("History"), Group::History),
        ("brush", tl!("Brushes"), Group::Brushes),
    ];
    for (icon, name, g) in entries {
        // Studio floats Properties outside the dock.
        let docked = t.pro || g != Group::Properties;
        let on = if customized { crate::panel_docking::rail_on(app, g) } else { g.shown(&p) && !(docked && app.ui.dock.is_collapsed(g)) };
        if icons::rail_button(ui, icon, rb, on, name).clicked() {
            if customized {
                crate::panel_docking::rail_click(app, g);
            } else {
                crate::dock::rail_click(app, g, docked);
            }
        }
    }
}

/// The dock's narrowest width (points). It is the right dock, on the left in a right-to-left UI.
pub(crate) const DOCK_MIN_WIDTH: f32 = 250.0;

/// What the dock leaves the toolbar and the canvas when it is widest (points).
const DOCK_LEAVES: f32 = 400.0;

/// The dock's width range (points) in a window `window_width` wide: from [`DOCK_MIN_WIDTH`] to
/// everything but [`DOCK_LEAVES`], and never capped below the old 520, so two-column layouts can
/// be as wide as the window allows (#7896).
pub(crate) fn dock_width_range(window_width: f32) -> std::ops::RangeInclusive<f32> {
    let max = if window_width.is_finite() { (window_width - DOCK_LEAVES).max(520.0) } else { 520.0 };
    DOCK_MIN_WIDTH..=max
}

pub(crate) fn dock_width_id() -> egui::Id {
    egui::Id::unique("dock-width-request")
}

/// Set the end dock's width on the next frame (`ui.set {dockWidth}`); clamped to its
/// window-dependent range. An external request supersedes an in-progress outer resize.
pub fn request_dock_width(ctx: &egui::Context, w: f32) {
    clear_dock_edge_drag(ctx);
    let range = dock_width_range(ctx.content_rect().width());
    let w = if w.is_finite() { w.clamp(*range.start(), *range.end()) } else { *range.start() };
    ctx.data_mut(|d| d.insert_temp(dock_width_id(), w));
}

/// One dock group's tab content; `panel_docking` bounds it and scrolls it when it's taller.
pub(crate) fn dock_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, group: crate::dock::Group, tab: usize) {
    use crate::dock::Group;
    let pro = Tokens::get(ui.ctx()).pro;
    match (group, tab) {
        (Group::Color, 2) => crate::preset_panels::gradients_panel(app, ui),
        (Group::Color, 3) => crate::preset_panels::patterns_panel(app, ui),
        // The Color tab is the same in every theme, in the mode picked from its panel menu
        // (#6000); only the tab order differs between themes.
        (Group::Color, tab) if group.tabs(pro).get(tab) == Some(&"Color") => crate::color_panel_ui::panel(app, ui),
        (Group::Color, _) => crate::swatches_ui::panel(app, ui),
        (Group::Properties, 0) => properties_body(app, ui),
        (Group::Properties, _) => adjustments_grid(app, ui),
        (Group::Character, tab) => crate::type_tool::character_panel(app, ui, tab == 1),
        (Group::Navigator, 0) => navigator(app, ui),
        (Group::Navigator, 1) => crate::histogram_panel::show(app, ui),
        (Group::Navigator, _) => info_panel(app, ui),
        (Group::History, 0) => history(app, ui),
        (Group::History, 1) => crate::actions::panel(app, ui),
        (Group::History, _) => crate::comps_ui::panel(app, ui),
        (Group::Layers, 0) => layers(app, ui),
        (Group::Layers, 1) => channels(app, ui),
        (Group::Layers, _) => crate::vector_ui::paths_panel(app, ui),
        (Group::Brushes, _) => crate::brushes_tab::show(app, ui),
    }
}
