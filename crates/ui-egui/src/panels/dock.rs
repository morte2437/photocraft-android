use super::history::history;
use super::info::info_panel;
use super::layers::{channels, layers};
use super::navigator::navigator;
use super::properties::{adjustments_grid, properties_body};
use super::{PhotocraftApp, Stroke, Tokens, icons, pos2, vec2};

// ----------------------------------------------------------------------------- dock

/// The panels and the canvas in the space left by the menus, bars and toolbar: craft-ui's
/// docking layout, with the icon rail at the window's end edge in the Studio themes.
pub fn dock_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if app.compact_layout {
        // The phone layout: the icon rail with its sheets, and the canvas beside it.
        phone_dock(app, ui);
        let t = Tokens::get(ui.ctx());
        let backdrop = if app.ui.view.hides_chrome() { egui::Color32::BLACK } else { crate::prefs_ui::pasteboard_color(app).unwrap_or(t.canvas) };
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(backdrop)).show(ui, |ui| crate::canvas::document_area(app, ui));
        return;
    }
    let t = Tokens::get(ui.ctx());
    if app.ui.panels.rail && !t.pro {
        rail(app, ui, &t);
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::panel_docking::show(app, ui));
    crate::dock::persist(app, ui.ctx());
    crate::color_panel_ui::persist(app);
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

/// Narrow icon rail, unless an embedding app hides it: shows, expands or collapses panel groups.
fn rail(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let customized = app.ui.docking.is_some();
    let p = app.ui.panels.clone();
    let (rw, rb) = if t.pro { (36.0, 28.0) } else { (44.0, 32.0) };
    crate::direction::end_panel("rail")
        .resizable(false)
        .exact_size(rw)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(4, 8)))
        .show(ui, |ui| {
            // The separator is on the canvas side.
            let r = ui.max_rect();
            let x = if crate::direction::is_rtl() { r.right() + 6.0 } else { r.left() - 6.0 };
            ui.painter().line_segment([pos2(x, r.top() - 8.0), pos2(x, r.bottom() + 8.0)], Stroke::new(1.0, t.separator));
            ui.spacing_mut().item_spacing.y = 4.0;
            use crate::dock::Group;
            for (icon, name, g) in rail_entries() {
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
        });
}

/// The rail's groups: icon, name, group.
fn rail_entries() -> [(&'static str, &'static str, crate::dock::Group); 6] {
    use crate::dock::Group;
    [
        ("sliders-horizontal", tl!("Properties"), Group::Properties),
        ("navigation", tl!("Navigator"), Group::Navigator),
        ("palette", tl!("Color & Swatches"), Group::Color),
        ("layers", tl!("Layers"), Group::Layers),
        ("clock", tl!("History"), Group::History),
        ("brush", tl!("Brushes"), Group::Brushes),
    ]
}

/// The phone layout's dock ([`PhotocraftApp::compact_layout`]): a side-by-side dock would take
/// most of a phone's width, so only the icon rail stays. A rail button opens its group as a
/// sheet over the canvas, one group at a time; the same button, Esc, or closing or collapsing
/// the group puts it away.
fn phone_dock(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    // Finger-sized buttons.
    let (rw, rb) = (44.0, 36.0);
    let rail = crate::direction::end_panel("rail")
        .resizable(false)
        .exact_size(rw)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(4, 8)))
        .show(ui, |ui| {
            let r = ui.max_rect();
            let x = if crate::direction::is_rtl() { r.right() + 4.0 } else { r.left() - 4.0 };
            ui.painter().line_segment([pos2(x, r.top() - 8.0), pos2(x, r.bottom() + 8.0)], Stroke::new(1.0, t.separator));
            ui.spacing_mut().item_spacing.y = 6.0;
            // Character too: there is no dock beside the canvas for it to appear in.
            let character = ("type", tl!("Character"), crate::dock::Group::Character);
            for (icon, name, g) in rail_entries().into_iter().chain([character]) {
                let on = app.phone_panel == Some(g);
                if icons::rail_button(ui, icon, rb, on, name).clicked() {
                    app.phone_panel = if on { None } else { Some(g) };
                    if !on {
                        crate::dock::reveal(app, g);
                    }
                }
            }
        })
        .response
        .rect;
    let Some(g) = app.phone_panel else { return };
    // A dialog previewing on the canvas (Layer Style, an adjustment, a filter) sits at the bottom
    // of the window; the sheet makes way so the image shows above it, and comes back after.
    if app.ui.dialogs.iter().any(crate::dialogs::previews_on_canvas) {
        return;
    }
    // Put away when its group was closed or collapsed from its own tab strip, or with Esc.
    if !g.shown(&app.ui.panels) || app.ui.dock.is_collapsed(g) || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        app.phone_panel = None;
        app.ui.dock.set_collapsed(g, false);
        return;
    }
    let screen = ui.ctx().content_rect();
    let w = (screen.width() - rw - 56.0).clamp(220.0, 360.0);
    let rect = if crate::direction::is_rtl() {
        egui::Rect::from_min_size(pos2(rail.right(), rail.top()), vec2(w, rail.height()))
    } else {
        egui::Rect::from_min_max(pos2(rail.left() - w, rail.top()), pos2(rail.left(), rail.bottom()))
    };
    egui::Area::new(egui::Id::unique("phone-panel-sheet")).order(egui::Order::Middle).fixed_pos(rect.min).constrain(false).show(ui.ctx(), |ui| {
        let frame = egui::Frame::NONE.fill(t.dock).stroke(Stroke::new(1.0, t.separator)).inner_margin(egui::Margin::same(2));
        frame.show(ui, |ui| {
            let inner = rect.size() - vec2(4.0, 4.0);
            ui.set_min_size(inner);
            ui.set_max_size(inner);
            phone_group(app, ui, g);
        });
    });
    crate::dock::persist(app, ui.ctx());
}

/// One panel group in the phone layout's sheet: its tabs, and the selected tab's body below.
fn phone_group(app: &mut PhotocraftApp, ui: &mut egui::Ui, g: crate::dock::Group) {
    let pro = Tokens::get(ui.ctx()).pro;
    let tabs = g.tabs(pro);
    let mut tab = (*g.tab_mut(&mut app.ui.dock_tabs)).min(tabs.len().saturating_sub(1));
    if tabs.len() > 1 {
        ui.horizontal_wrapped(|ui| {
            for (i, name) in tabs.iter().enumerate() {
                if crate::widgets::pill_tab(ui, tl!(name), i == tab).clicked() {
                    tab = i;
                }
            }
        });
        ui.add_space(4.0);
    }
    *g.tab_mut(&mut app.ui.dock_tabs) = tab;
    if g.scrolls_itself(tab) {
        ui.set_min_height(ui.available_height().max(0.0));
        dock_body(app, ui, g, tab);
    } else {
        ui.spacing_mut().scroll.fade.strength = 0.0;
        egui::ScrollArea::vertical().id_salt(("phone-panel-body", g)).auto_shrink([false, false]).show(ui, |ui| dock_body(app, ui, g, tab));
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

/// Set the right dock's width on the next frame (`ui.set {dockWidth}`); clamped to its range.
pub fn request_dock_width(ctx: &egui::Context, w: f32) {
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
