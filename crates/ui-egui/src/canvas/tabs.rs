use super::start_screen::start_screen;
use super::textures::retain_gpu_documents;
use super::{Color32, PhotocraftApp, Pos2, Rect, Sense, Stroke, TextureOptions, canvas_view, fmt_zoom, json, mode_label, pos2};

/// Tabs + canvas for the active document, or the start screen; then drops layers dragged onto
/// another document (`layer_transfer`).
pub fn document_area(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::distort_ui::drop_foreign(app);
    documents(app, ui);
    crate::layer_transfer::finish(app, ui.ctx());
    // Once a frame, not per document view: the right-click or options-bar Brush Preset picker.
    crate::paint_mouse::show_picker(app, ui.ctx());
    // And the Shift+Alt+right-click HUD Color Picker (#6098), while its gesture is held.
    crate::color_hud::show(app, ui.ctx());
    crate::brush_mode_menu::show(app, ui.ctx());
}

pub(crate) fn arranged_cells(app: &PhotocraftApp, rect: Rect) -> Option<Vec<(usize, Rect)>> {
    let documents = docked_document_indices(app);
    let n = documents.len();
    if n == 0 {
        return None;
    }
    let cells = crate::view_cmds::cells(&app.ui.view.arrange, rect, n)?;
    // Keep stable tiles, expressing the remembered document index in the filtered docked list.
    let start = documents.partition_point(|&d| d < app.arrange_start).min(n.saturating_sub(cells.len()));
    let idx = app.session.active_index().and_then(|d| documents.iter().position(|&p| p == d));
    let start = idx.map_or(start, |idx| if idx < start { idx } else { start.max((idx + 1).saturating_sub(cells.len())) });
    Some(documents[start..].iter().copied().zip(cells).collect())
}

fn documents(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    app.drop_canvas_rect = None;
    app.tab_strip = None;
    retain_gpu_documents(app);
    crate::transform_tool::track_steps(app, ui.ctx());
    crate::cas_transform::tick(app, ui.ctx());
    crate::symmetry_ui::track(app, ui.ctx());
    let n = app.session.documents().len();
    // Files opening in the background (#210) have tabs before they have documents.
    let opening = !app.jobs.opens.is_empty();
    if !opening && app.ui.chrome.shows_home(n, app.session.prefs().general.auto_show_home_screen) {
        start_screen(app, ui);
        return;
    }
    if n == 0 && !opening {
        // Auto show the Home Screen is off: an empty workspace, like Photoshop.
        let area = ui.available_rect_before_wrap();
        paint_dots(ui, area, app.session.prefs().interface.show_canvas_dots);
        open_on_double_click(app, ui, area);
        return;
    }
    if n > 0 && !opening && docked_document_indices(app).is_empty() {
        // A torn-off document belongs to its OS window. Leaving its canvas in the main
        // viewport would make the same page appear twice, which is what “New Window” does.
        paint_dots(ui, ui.available_rect_before_wrap(), app.session.prefs().interface.show_canvas_dots);
        return;
    }
    if !app.ui.view.hides_tabs() || opening {
        // Flat Spectrum tabs sit directly against the canvas. The parent's default vertical
        // item spacing would otherwise leave a visible gap below the tab strip (#2188).
        let spacing = ui.spacing().item_spacing.y;
        if crate::theme::Tokens::get(ui.ctx()).pro {
            ui.spacing_mut().item_spacing.y = 0.0;
        }
        app.tab_strip = Some(tabs(app, ui));
        ui.spacing_mut().item_spacing.y = spacing;
        drop_slot_line(app, ui);
    }
    if let Some(job) = app.jobs.focus.or_else(|| (n == 0).then(|| app.jobs.opens.last().map(|o| o.job)).flatten()) {
        crate::jobs_ui::open_card(app, ui, job);
        return;
    }
    let Some(active_doc) = app.session.active_index() else { return };
    let Some(idx) = main_canvas_document(app) else {
        paint_dots(ui, ui.available_rect_before_wrap(), app.session.prefs().interface.show_canvas_dots);
        return;
    };
    let main_focused = ui.ctx().input(|i| i.viewport().focused.unwrap_or(true));
    let rect = ui.available_rect_before_wrap();
    app.last_canvas_rect = rect;
    app.drop_canvas_rect = Some(rect);
    // Window › Arrange: tiled / n-up layouts show several documents side by side; the active one
    // takes input, a click elsewhere activates that document.
    if let Some(shown) = arranged_cells(app, rect) {
        app.arrange_start = shown.first().map_or(0, |(d, _)| *d);
        let t = crate::theme::Tokens::get(ui.ctx());
        for (d, cell) in shown {
            let cell = cell.shrink(1.0);
            let view = app.ui.views[d].clone();
            let v = canvas_view(app, ui, d, cell, view, d == active_doc);
            if d != active_doc {
                app.ui.views[d] = v;
                if main_focused && ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| cell.contains(p))) {
                    app.session.set_active(d);
                }
            }
            let stroke = if d == active_doc { Stroke::new(1.5, t.accent) } else { Stroke::new(1.0, Color32::from_black_alpha(160)) };
            ui.painter().rect_stroke(cell, 0, stroke, egui::StrokeKind::Inside);
        }
        return;
    }
    if idx != active_doc && main_focused && ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| rect.contains(p))) {
        app.session.set_active(idx);
    }
    let view = app.ui.views[idx].clone();
    canvas_view(app, ui, idx, rect, view, app.session.active_index() == Some(idx));
}

/// The tab strip; returns where its document tabs are.
pub(super) fn tabs(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> TabStrip {
    let t = crate::theme::Tokens::get(ui.ctx());
    if t.pro {
        return pro_tabs(app, ui);
    }
    let active = app.session.active_index();
    let documents = docked_document_indices(app);
    let tab_count = documents.len();
    let document_count = app.session.documents().len();
    let mut doc_tabs = Vec::with_capacity(tab_count);
    let mut activate = None;
    let mut close = None;
    let mut tab_action = None;
    let mut tab_move = None;
    let mut tear_off = None;
    let (mut focus_open, mut cancel_open) = (None, None);
    let focused_open = app.jobs.focus.is_some();
    // Layers dragged over a tab show its document (`layer_transfer`).
    let dragging = crate::layer_transfer::pointer_if_armed(app, ui.ctx());
    let mut drag_over = None;
    let large_tabs = app.session.prefs().workspace.large_tabs;
    let tab_h = if large_tabs { 34.0 } else { 26.0 };
    let font = crate::theme::medium(if large_tabs { 14.0 } else { 12.5 });
    let meta_font = egui::FontId::proportional(if large_tabs { 12.0 } else { 10.5 });
    // Files opening in the background (#210) are tabs too; they share the fit's index space,
    // after the documents.
    let opening = crate::jobs_ui::open_tabs(app);
    // Every tab's natural width, whether or not it stays on the strip.
    let natural: Vec<f32> = documents
        .iter()
        .filter_map(|&doc| app.session.documents().get(doc))
        .map(|st| {
            let name = studio_tab_name(app, st);
            ui.painter().layout_no_wrap(name, font.clone(), t.text).size().x
                + ui.painter().layout_no_wrap(format!("{}/{}", mode_label(&st.doc), st.doc.depth.bits()), meta_font.clone(), t.text_faint).size().x
                + STUDIO_TAB_PAD
        })
        .chain(opening.iter().map(|(_, name, frac)| {
            ui.painter().layout_no_wrap(name.clone(), font.clone(), t.text).size().x
                + ui.painter().layout_no_wrap(format!("{:.0}%", frac * 100.0), meta_font.clone(), t.text_faint).size().x
                + STUDIO_TAB_PAD
        }))
        .collect();
    let selected = app
        .jobs
        .focus
        .and_then(|job| opening.iter().position(|(open, _, _)| *open == job))
        .map_or_else(|| app.session.active_index().and_then(|doc| documents.iter().position(|&d| d == doc)).unwrap_or(0), |p| tab_count + p);
    let frame = egui::Frame::NONE.fill(t.canvas).inner_margin(egui::Margin { left: 8, right: 8, top: 6, bottom: 4 }).show(ui, |ui| {
        let (row, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), tab_h), Sense::hover());
        let f = fit_doc_tabs(&natural, selected, row.width());
        let mut x = row.left();
        let mut placed: Vec<(usize, Rect)> = Vec::with_capacity(f.shown.len());
        for &(i, w) in &f.shown {
            // The old horizontal layout put 4 pt between tabs; `w` carries it, the tab doesn't.
            let r = Rect::from_min_size(egui::pos2(x, row.top()), egui::vec2((w - STUDIO_TAB_GAP).max(8.0), row.height()));
            placed.push((i, r));
            x = r.right() + STUDIO_TAB_GAP;
        }
        for &(i, r) in placed.iter().filter(|(i, _)| *i < tab_count) {
            let Some(&doc) = documents.get(i) else { continue };
            let Some(st) = app.session.documents().get(doc) else { continue };
            let sel = Some(doc) == active && !focused_open;
            let name = studio_tab_name(app, st);
            let meta = format!("{}/{}", mode_label(&st.doc), st.doc.depth.bits());
            let natural_w = natural.get(i).copied().unwrap_or(0.0);
            let meta_g = ui.painter().layout_no_wrap(meta, meta_font.clone(), t.text_faint);
            let name_max = (r.width() - STUDIO_TAB_PAD + STUDIO_TAB_GAP - meta_g.size().x).max(1.0);
            let name_g = crate::tab_strip::elided(ui, &name, font.clone(), t.text, name_max);
            // The title was cut: the tooltip has the rest of it.
            let cut = name_g.size().x + 0.5 < natural_w - STUDIO_TAB_PAD + STUDIO_TAB_GAP - meta_g.size().x;
            let resp = ui.interact(r, ui.scope_id().with(("dtab", doc)), Sense::click_and_drag());
            let saving = crate::jobs_ui::save_progress(app, st.doc.id);
            resp.widget_info(|| {
                let label = saving.as_ref().map_or_else(|| st.doc.name.clone(), |s| format!("{} - {s}", st.doc.name));
                egui::WidgetInfo::selected(egui::Role::Button, true, sel, label)
            });
            doc_tabs.push((doc, r));
            if sel {
                ui.painter().rect_filled(r, t.radius_sm, t.card);
                ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, t.radius_sm, t.hover.gamma_multiply(0.5));
            }
            if dragging.is_some_and(|p| r.contains(p)) {
                drag_over = Some(doc);
                ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            }
            let color = if sel { t.text } else { t.text_dim };
            let ny = r.center().y - name_g.size().y / 2.0;
            ui.painter().galley_with_override_text_color(egui::pos2(r.left() + 10.0, ny), name_g.clone(), color);
            ui.painter().galley(egui::pos2(r.left() + 16.0 + name_g.size().x, r.center().y - meta_g.size().y / 2.0), meta_g, t.text_faint);
            let xr = Rect::from_center_size(egui::pos2(r.right() - 12.0, r.center().y), egui::vec2(16.0, 16.0));
            let xresp = ui.interact(xr, ui.scope_id().with(("dtabx", doc)), Sense::click());
            if xresp.hovered() {
                ui.painter().rect_filled(xr, 4.0, t.hover);
            }
            crate::icons::paint(ui, xr, "x", 11.0, if xresp.hovered() { t.text } else { t.text_faint });
            let resp = if cut { resp.on_hover_text(name) } else { resp };
            if tab_close_clicked(&resp, &xresp) {
                close = Some(doc);
            } else if resp.clicked() {
                activate = Some(doc);
            }
            match tab_drag(ui, &resp, i, r, row, &placed, &t) {
                Some(TabDrop::Move(to)) => tab_move = Some((doc, move_target_after_filter(&documents, doc, to, document_count))),
                Some(TabDrop::Float(at)) => tear_off = Some((doc, at)),
                None => {}
            }
            let has_path = st.path.is_some();
            resp.context_menu(|ui| {
                tab_action = tab_context_menu(ui, doc, document_count, has_path);
            });
        }
        // Files opening in the background: a tab with a progress underline; × cancels.
        for &(i, r) in placed.iter().filter(|(i, _)| *i >= tab_count) {
            let Some((job, name, frac)) = opening.get(i - tab_count) else { continue };
            let sel = app.jobs.focus == Some(*job);
            let meta_g = ui.painter().layout_no_wrap(format!("{:.0}%", frac * 100.0), meta_font.clone(), t.text_faint);
            let name_g = crate::tab_strip::elided(ui, name, font.clone(), t.text, (r.width() - STUDIO_TAB_PAD + STUDIO_TAB_GAP - meta_g.size().x).max(1.0));
            let resp = ui.interact(r, ui.scope_id().with(("dtabjob", job.0)), Sense::click());
            if sel {
                ui.painter().rect_filled(r, t.radius_sm, t.card);
                ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.card_border), egui::StrokeKind::Inside);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, t.radius_sm, t.hover.gamma_multiply(0.5));
            }
            let ny = r.center().y - name_g.size().y / 2.0;
            ui.painter().galley_with_override_text_color(egui::pos2(r.left() + 10.0, ny), name_g.clone(), if sel { t.text } else { t.text_dim });
            ui.painter().galley(egui::pos2(r.left() + 16.0 + name_g.size().x, r.center().y - meta_g.size().y / 2.0), meta_g, t.text_faint);
            crate::jobs_ui::tab_underline(ui, r, *frac, &t);
            let xr = Rect::from_center_size(egui::pos2(r.right() - 12.0, r.center().y), egui::vec2(16.0, 16.0));
            let xresp = ui.interact(xr, ui.scope_id().with(("dtabjobx", job.0)), Sense::click());
            if xresp.hovered() {
                ui.painter().rect_filled(xr, 4.0, t.hover);
            }
            crate::icons::paint(ui, xr, "x", 11.0, if xresp.hovered() { t.text } else { t.text_faint });
            if xresp.on_hover_text(tl!("Cancel opening")).clicked() {
                cancel_open = Some(*job);
            } else if resp.clicked() {
                focus_open = Some(*job);
            }
        }
        if !f.overflow.is_empty() {
            // Pinned to the strip's right end, as Photoshop's is, not after the last tab (#8290).
            let left = (row.right() - crate::tab_strip::CHEVRON_W).max(row.left());
            let r = Rect::from_min_size(egui::pos2(left, row.top()), egui::vec2(crate::tab_strip::CHEVRON_W, row.height()));
            let labels: Vec<&str> = documents
                .iter()
                .filter_map(|&doc| app.session.documents().get(doc).map(|st| st.doc.name.as_str()))
                .chain(opening.iter().map(|(_, name, _)| name.as_str()))
                .collect();
            let mut picked = None;
            // Like Photoshop's, the menu lists every open document, not only the hidden ones, and checks
            // the active one (#4487).
            let every: Vec<usize> = (0..labels.len()).collect();
            crate::tab_strip::overflow_button(ui, ui.scope_id().with("dtab-overflow"), r, tl!("More documents"), &labels, &every, Some(selected), &mut picked);
            match picked {
                Some(i) if i < tab_count => activate = documents.get(i).copied(),
                Some(i) => focus_open = opening.get(i - tab_count).map(|(job, _, _)| *job).or(focus_open),
                None => {}
            }
        }
    });
    open_tab_clicks(app, activate, focus_open, cancel_open);
    if let Some(i) = drag_over {
        crate::layer_transfer::over_tab(app, ui.ctx(), i);
    }
    if let Some(i) = close {
        let _ = crate::menus::invoke(app, ui.ctx(), "file.close", json!({"document": i}));
    }
    if let Some((from, to)) = tab_move
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), "document.move", json!({"document": from, "to": to}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    float_torn_off(app, ui.ctx(), tear_off);
    if let Some((id, params)) = tab_action
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), id, params)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    TabStrip { rect: frame.response.rect, tabs: doc_tabs }
}

/// Whether a document tab was asked to close: its × was clicked (#619), or the tab was clicked
/// with the middle button (#2570), as in a browser. A middle click over the × counts too, so the
/// whole tab behaves the same way. Both strips share this rule; the caller routes it through the
/// guarded `file.close`, so a document with unsaved changes still gets the save prompt.
fn tab_close_clicked(resp: &egui::Response, xresp: &egui::Response) -> bool {
    xresp.clicked() || xresp.middle_clicked() || resp.middle_clicked()
}

/// All tab actions go through the same guarded File commands as the menu bar, including the
/// unsaved-changes prompt. The clicked tab is explicit even when another document is active.
pub(super) fn tab_context_items(index: usize, count: usize, has_path: bool) -> [(&'static str, &'static str, serde_json::Value, bool); 4] {
    [
        ("Close", "file.close", json!({"document": index}), true),
        ("Close Others", "file.closeOthers", json!({"document": index}), count > 1),
        ("Close All", "file.closeAll", json!({}), true),
        (crate::menus::reveal_label(), "file.revealInFinder", json!({"document": index}), has_path),
    ]
}

fn tab_context_menu(ui: &mut egui::Ui, index: usize, count: usize, has_path: bool) -> Option<(&'static str, serde_json::Value)> {
    crate::widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(170.0);
        for (label, id, params, enabled) in tab_context_items(index, count, has_path) {
            if ui.add_enabled(enabled, egui::Button::new(tl!(label))).clicked() {
                ui.close();
                return Some((id, params));
            }
        }
        None
    })
}

/// Where a tab dragged to `x` would land: before the first shown tab whose middle is right of
/// `x`, else after the last shown tab (its document index + 1), like [`TabStrip::slot`].
/// How far (points) above or below the tab strip a dragged tab must be to tear off (#3733).
pub(super) const TEAR_OFF: f32 = 24.0;

/// Whether a tab dragged to `p` has left the strip and floats into its own window, as in
/// Photoshop; sideways drags along the strip still reorder (#217).
pub(super) fn torn_off(strip: Rect, p: egui::Pos2) -> bool {
    p.y > strip.bottom() + TEAR_OFF || p.y < strip.top() - TEAR_OFF
}

/// What a finished document-tab drag asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TabDrop {
    /// Reorder the tab to this index.
    Move(usize),
    /// Float the document in its own OS window at this screen position (points), when known.
    Float(Option<[f32; 2]>),
}

/// Drag a document tab: along the strip it reorders (a line shows where it will land); off the
/// strip it tears off into a floating window that can go anywhere, other screens included
/// (an outline under the pointer shows it). Returns the drop once the drag ends.
fn tab_drag(ui: &egui::Ui, resp: &egui::Response, i: usize, r: Rect, strip: Rect, placed: &[(usize, Rect)], t: &crate::theme::Tokens) -> Option<TabDrop> {
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        if torn_off(strip, p) {
            let ghost = Rect::from_min_size(p - egui::vec2(40.0, 12.0), egui::vec2(240.0, 160.0));
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.scope_id().with("tab-tear-off")));
            painter.rect_filled(ghost, t.radius_sm, t.accent.gamma_multiply(0.12));
            painter.rect_stroke(ghost, t.radius_sm, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        } else {
            tab_drop_line(ui, placed, p.x, t);
        }
    }
    if !resp.drag_stopped() {
        return None;
    }
    let p = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.interact_pos()));
    if let Some(p) = p.filter(|&p| torn_off(strip, p)) {
        // The pointer is in this window's points; the OS wants screen coordinates. Keep the new
        // window's title bar under the pointer, as the ghost showed.
        let at = ui.ctx().input(|i| i.viewport().inner_rect).map(|inner| {
            let s = inner.min + p.to_vec2() - egui::vec2(40.0, 12.0);
            [s.x, s.y]
        });
        return Some(TabDrop::Float(at));
    }
    let to = tab_move_target(i, tab_drop_index(placed, p.map_or(r.center().x, |p| p.x)));
    (to != i).then_some(TabDrop::Move(to))
}

/// Float a torn-off tab's document in its own window (`window.arrange.floatInWindow`).
fn float_torn_off(app: &mut PhotocraftApp, ctx: &egui::Context, tear_off: Option<(usize, Option<[f32; 2]>)>) {
    let Some((doc, at)) = tear_off else { return };
    let params = match at {
        Some(at) => json!({"document": doc, "position": at}),
        None => json!({"document": doc}),
    };
    if let Err(e) = crate::menus::invoke(app, ctx, "window.arrange.floatInWindow", params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

pub(super) fn tab_drop_index(placed: &[(usize, Rect)], x: f32) -> usize {
    match placed.iter().find(|(_, r)| r.center().x >= x) {
        Some(&(i, _)) => i,
        None => placed.last().map_or(0, |&(i, _)| i.saturating_add(1)),
    }
}

/// The `document.move` target for dropping the tab at `from` at insertion index `k`: the slot
/// shifts down when the dragged tab itself is removed first; next to itself it stays put.
pub(super) fn tab_move_target(from: usize, k: usize) -> usize {
    if k > from { k - 1 } else { k }
}

/// The insertion line shown while a tab is dragged to reorder it.
fn tab_drop_line(ui: &egui::Ui, placed: &[(usize, Rect)], x: f32, t: &crate::theme::Tokens) {
    match placed.iter().find(|(_, r)| r.center().x >= x) {
        Some(&(_, r)) => crate::widgets::drop_line(ui, r, false, true, t),
        None => {
            if let Some(&(_, r)) = placed.last() {
                crate::widgets::drop_line(ui, r, true, true, t);
            }
        }
    }
}

/// Apply tab-strip clicks: a document tab shows that document, an opening tab its progress, and
/// an opening tab's × cancels the open (closing the half-open tab).
fn open_tab_clicks(
    app: &mut PhotocraftApp,
    activate: Option<usize>,
    focus_open: Option<photocraft_engine::jobs::JobId>,
    cancel_open: Option<photocraft_engine::jobs::JobId>,
) {
    if let Some(i) = activate {
        app.session.set_active(i);
        app.jobs.focus = None;
    }
    if let Some(job) = focus_open {
        app.jobs.focus = Some(job);
    }
    if let Some(job) = cancel_open {
        crate::jobs_ui::cancel(app, job);
    }
}

/// Document tabs: the chrome around a title (the × and its gaps).
const DOC_TAB_PAD: f32 = 42.0;

/// The Studio strip's tabs carried 4 pt of spacing between them and 44 pt of chrome; `fit` sees
/// one number per tab, so the gap rides in the natural width and comes back out when painted.
const STUDIO_TAB_PAD: f32 = 48.0;

const STUDIO_TAB_GAP: f32 = 4.0;

/// Fit document tabs as Photoshop does (#4487): a tab never shrinks below its whole title, so the
/// tabs that don't fit move into the » menu instead. Only the active tab, when it alone is wider
/// than the strip, is cut.
fn fit_doc_tabs(natural: &[f32], selected: usize, avail: f32) -> crate::tab_strip::TabFit {
    let widest = natural.iter().copied().filter(|w| w.is_finite()).fold(0.0, f32::max);
    crate::tab_strip::fit(natural, selected, avail, widest, crate::tab_strip::CHEVRON_W)
}

/// Photoshop's tab title: "name @ 50% (Layer 1, RGB/8)", "(Layer 1, Layer Mask/8)" when the mask
/// is targeted, "(Quick Mask/8)" in Quick Mask mode; the Background layer's name is omitted.
pub(super) fn pro_tab_title(app: &PhotocraftApp, i: usize) -> String {
    tab_title_at(app, i, app.ui.views.get(i).map_or(100.0, |v| v.zoom * 100.0))
}

/// [`pro_tab_title`] at `zoom` percent (a workspace with its own view, such as Select and Mask).
pub(crate) fn tab_title_at(app: &PhotocraftApp, i: usize, zoom: f32) -> String {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let Some(st) = app.session.documents().get(i) else { return String::new() };
    // Quick Mask mode targets the mask, not the layer (see `paint_target`): painting and Free
    // Transform change the red overlay, so the tab says so, as Photoshop's does.
    let quick_mask = st.doc.quick_mask.is_some() && st.channel_view.target == ChannelTarget::Composite;
    let active_layer = st.active_layer.and_then(|id| st.doc.layer(id)).filter(|l| !quick_mask && !st.doc.is_background(l.id));
    let mask = app.session.active_index() == Some(i) && app.ui.mask_target && active_layer.is_some_and(|l| l.mask.is_some());
    let model = if quick_mask {
        tl!("Quick Mask").to_string()
    } else if mask {
        tl!("Layer Mask").to_string()
    } else {
        mode_label(&st.doc).to_string()
    };
    let layer = active_layer.map(|l| format!("{}, ", l.name)).unwrap_or_default();
    // A save in progress: "name @ 100% (Layer 1, RGB/16) - Saving 45%", as Photoshop does (#3389).
    let saving = crate::jobs_ui::save_progress(app, st.doc.id).map(|s| format!(" - {s}")).unwrap_or_default();
    format!("{} @ {}% ({layer}{model}/{}){}{saving}", st.doc.name, fmt_zoom(zoom), st.doc.depth.bits(), if st.is_dirty() { "*" } else { "" })
}

/// The Studio strip's tab name: "name *" when unsaved, "name * - Saving 45%" while it saves (#3389).
fn studio_tab_name(app: &PhotocraftApp, st: &photocraft_engine::DocState) -> String {
    let saving = crate::jobs_ui::save_progress(app, st.doc.id).map(|s| format!(" - {s}")).unwrap_or_default();
    format!("{}{}{saving}", st.doc.name, if st.is_dirty() { " *" } else { "" })
}

/// Photoshop document tabs: "name @ 33.3% (RGB/8)" on a dark strip; active tab matches panels.
/// Tabs keep their whole titles, and the ones that don't fit move into a » menu at the end of the
/// strip; the active document's tab always stays on the strip (#1276, #4487).
fn pro_tabs(app: &mut PhotocraftApp, ui: &mut egui::Ui) -> TabStrip {
    let t = crate::theme::Tokens::get(ui.ctx());
    let documents = docked_document_indices(app);
    let active_doc = app.session.active_index().filter(|_| app.jobs.focus.is_none());
    let active = active_doc.and_then(|doc| documents.iter().position(|&d| d == doc));
    let (mut activate, mut close) = (None, None);
    let mut tab_action = None;
    let mut tab_move = None;
    let mut tear_off = None;
    let tab_count = documents.len();
    let document_count = app.session.documents().len();
    let dragging = crate::layer_transfer::pointer_if_armed(app, ui.ctx());
    let mut drag_over = None;
    let mac = ui.ctx().os() == egui::os::OperatingSystem::Mac;
    let large_tabs = app.session.prefs().workspace.large_tabs;
    let font = egui::FontId::proportional(if large_tabs { 13.0 } else { 11.5 });
    let tab_h = if large_tabs { 34.0 } else { 26.0 };
    let (strip, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), tab_h), Sense::hover());
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    // Files opening in the background (#210) are tabs too ("name (Opening… 45%)" with a progress
    // underline); they share the fit's index space, after the documents.
    let opening = crate::jobs_ui::open_tabs(app);
    let mut titles: Vec<String> = documents.iter().map(|&i| pro_tab_title(app, i)).collect();
    titles.extend(opening.iter().map(|(_, name, frac)| format!("{name} ({} {:.0}%)", tl!("Opening…"), frac * 100.0)));
    // Every tab's natural width, whether or not it stays on the strip.
    let natural: Vec<f32> = titles.iter().map(|title| ui.painter().layout_no_wrap(title.clone(), font.clone(), t.text).size().x + DOC_TAB_PAD).collect();
    let selected = app.jobs.focus.and_then(|job| opening.iter().position(|(open, _, _)| *open == job)).map_or_else(|| active.unwrap_or(0), |p| tab_count + p);
    let f = fit_doc_tabs(&natural, selected, strip.width());
    let mut x = strip.left();
    let mut placed: Vec<(usize, Rect)> = Vec::with_capacity(f.shown.len());
    for &(i, w) in &f.shown {
        let r = Rect::from_min_size(egui::pos2(x, strip.top()), egui::vec2(w, strip.height()));
        placed.push((i, r));
        x = r.right();
    }
    let mut doc_tabs = Vec::with_capacity(tab_count);
    let (mut focus_open, mut cancel_open) = (None, None);
    for &(i, r) in placed.iter().filter(|(i, _)| *i < tab_count) {
        let Some(&doc) = documents.get(i) else { continue };
        let Some(title) = titles.get(i) else { continue };
        let g = crate::tab_strip::elided(ui, title, font.clone(), t.text, (r.width() - DOC_TAB_PAD).max(1.0));
        let resp = ui.interact(r, ui.scope_id().with(("ptab", doc)), Sense::click_and_drag());
        let sel = Some(i) == active;
        if sel {
            ui.painter().rect_filled(r, 0.0, t.chrome);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.35));
        }
        if dragging.is_some_and(|p| r.contains(p)) {
            drag_over = Some(doc);
            ui.painter().rect_stroke(r, 0.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
        }
        ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.0, t.separator));
        let (xr, at) = pro_tab_layout(r, g.size().y, mac);
        let xresp = ui.interact(xr, ui.scope_id().with(("ptabx", doc)), Sense::click());
        // Painted: name the tab and its × for accessibility (and so tests and agents can find them).
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, sel, g.text()));
        xresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Close")));
        crate::icons::paint(ui, xr, "x", 10.0, if xresp.hovered() { t.text } else { t.text_faint });
        ui.painter().galley_with_override_text_color(at, g, if sel { t.text } else { t.text_faint });
        let cut = natural.get(i).copied().unwrap_or(0.0) > r.width() + 0.5;
        let resp = if cut { resp.on_hover_text(title.as_str()) } else { resp };
        if tab_close_clicked(&resp, &xresp) {
            close = Some(doc);
        } else if resp.clicked() {
            activate = Some(doc);
        }
        match tab_drag(ui, &resp, i, r, strip, &placed, &t) {
            Some(TabDrop::Move(to)) => tab_move = Some((doc, move_target_after_filter(&documents, doc, to, document_count))),
            Some(TabDrop::Float(at)) => tear_off = Some((doc, at)),
            None => {}
        }
        let has_path = app.session.documents().get(doc).is_some_and(|d| d.path.is_some());
        resp.context_menu(|ui| {
            tab_action = tab_context_menu(ui, doc, document_count, has_path);
        });
        doc_tabs.push((doc, r));
    }
    for &(i, r) in placed.iter().filter(|(i, _)| *i >= tab_count) {
        let Some((job, _, frac)) = opening.get(i - tab_count) else { continue };
        let Some(title) = titles.get(i) else { continue };
        let g = crate::tab_strip::elided(ui, title, font.clone(), t.text, (r.width() - DOC_TAB_PAD).max(1.0));
        let resp = ui.interact(r, ui.scope_id().with(("ptabjob", job.0)), Sense::click());
        let sel = app.jobs.focus == Some(*job);
        if sel {
            ui.painter().rect_filled(r, 0.0, t.chrome);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.35));
        }
        ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.0, t.separator));
        crate::jobs_ui::tab_underline(ui, r, *frac, &t);
        let (xr, at) = pro_tab_layout(r, g.size().y, mac);
        let xresp = ui.interact(xr, ui.scope_id().with(("ptabjobx", job.0)), Sense::click());
        xresp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Close")));
        crate::icons::paint(ui, xr, "x", 10.0, if xresp.hovered() { t.text } else { t.text_faint });
        ui.painter().galley_with_override_text_color(at, g, if sel { t.text } else { t.text_faint });
        if xresp.on_hover_text(tl!("Cancel opening")).clicked() {
            cancel_open = Some(*job);
        } else if resp.clicked() {
            focus_open = Some(*job);
        }
    }
    if !f.overflow.is_empty() {
        // Pinned to the strip's right end, as Photoshop's is, not after the last tab (#8290).
        let left = (strip.right() - crate::tab_strip::CHEVRON_W).max(strip.left());
        let r = Rect::from_min_size(egui::pos2(left, strip.top()), egui::vec2(crate::tab_strip::CHEVRON_W, strip.height()));
        let labels: Vec<&str> = titles.iter().map(String::as_str).collect();
        let mut picked = None;
        // Like Photoshop's, the menu lists every open document, not only the hidden ones, and checks
        // the active one (#4487).
        let every: Vec<usize> = (0..labels.len()).collect();
        crate::tab_strip::overflow_button(ui, ui.scope_id().with("ptab-overflow"), r, tl!("More documents"), &labels, &every, Some(selected), &mut picked);
        match picked {
            Some(i) if i < tab_count => activate = documents.get(i).copied(),
            Some(i) => focus_open = opening.get(i - tab_count).map(|(job, _, _)| *job).or(focus_open),
            None => {}
        }
    }
    open_tab_clicks(app, activate, focus_open, cancel_open);
    if let Some(i) = drag_over {
        crate::layer_transfer::over_tab(app, ui.ctx(), i);
    }
    if let Some(i) = close {
        let _ = crate::menus::invoke(app, ui.ctx(), "file.close", json!({"document": i}));
    }
    if let Some((from, to)) = tab_move
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), "document.move", json!({"document": from, "to": to}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    float_torn_off(app, ui.ctx(), tear_off);
    if let Some((id, params)) = tab_action
        && let Err(e) = crate::menus::invoke(app, ui.ctx(), id, params)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    TabStrip { rect: strip, tabs: doc_tabs }
}

/// Where the document tabs were drawn (opening files' tabs aren't slots): a file dropped on the
/// strip opens at the slot under the pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct TabStrip {
    pub rect: Rect,
    /// The document tabs shown, left to right, with their document index. When the tabs overflow
    /// into the » menu, some documents have no tab here.
    pub tabs: Vec<(usize, Rect)>,
}

impl TabStrip {
    /// The document position a drop at `x` opens at: before the first shown tab whose middle is
    /// right of it, else after the last shown tab.
    pub fn slot(&self, x: f32) -> usize {
        match self.tabs.iter().find(|(_, r)| r.center().x >= x) {
            Some(&(i, _)) => i,
            None => self.tabs.last().map_or(0, |&(i, _)| i.saturating_add(1)),
        }
    }
}

/// While files are dragged over the tab strip, an insertion line where they would open.
fn drop_slot_line(app: &mut PhotocraftApp, ui: &egui::Ui) {
    if ui.input(|i| i.raw.hovered_files.is_empty()) {
        return;
    }
    let at = app.services.cursor_pos.as_mut().and_then(|f| f(ui.ctx()));
    let crate::file_open::DropTarget::Tabs(slot) = app.drop_target(ui.ctx(), at) else { return };
    let Some(tabs) = app.tab_strip.as_ref().map(|s| &s.tabs) else { return };
    let Some((r, after)) = tabs.iter().find(|(i, _)| *i == slot).map(|&(_, r)| (r, false)).or_else(|| tabs.last().map(|&(_, r)| (r, true))) else {
        return;
    };
    crate::widgets::drop_line(ui, r, after, true, &crate::theme::Tokens::get(ui.ctx()));
}

/// A Photoshop-style tab's close button and where its title starts. Photoshop puts the × after the
/// title on Windows (and so Linux) and before it on macOS (#619).
fn pro_tab_layout(tab: Rect, title_height: f32, mac: bool) -> (Rect, egui::Pos2) {
    let (x, title) = if mac { (tab.left() + 13.0, tab.left() + 26.0) } else { (tab.right() - 13.0, tab.left() + 16.0) };
    (Rect::from_center_size(egui::pos2(x, tab.center().y), egui::vec2(14.0, 14.0)), egui::pos2(title, tab.center().y - title_height / 2.0))
}

/// Repeating dot-grid texture for the canvas surround. A theme whose dots are its canvas colour
/// has no pattern to show, so it gets no texture at all rather than an invisible one (#4542).
pub(super) fn dots(ctx: &egui::Context, t: &crate::theme::Tokens) -> Option<egui::TextureId> {
    if t.bevel || t.canvas_dot == t.canvas {
        return None;
    }
    let key = egui::Id::unique(("canvas-dots", format!("{:?}", t.kind)));
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(key)) {
        return Some(tex.id());
    }
    let n = 22usize;
    let mut px = vec![Color32::TRANSPARENT; n * n];
    for (dx, dy, a) in [(0usize, 0usize, 255u8), (1, 0, 110), (0, 1, 110), (1, 1, 60)] {
        let c = t.canvas_dot;
        px[(n / 2 + dy) * n + n / 2 + dx] = Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a);
    }
    let opts = TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::Repeat,
        mipmap_mode: None,
    };
    let tex = ctx.load_texture("canvas-dots", egui::ColorImage::new([n, n], px), opts);
    let id = tex.id();
    ctx.data_mut(|d| d.insert_temp(key, tex));
    Some(id)
}

pub(super) fn paint_dots(ui: &egui::Ui, rect: Rect, enabled: bool) {
    if !enabled {
        return;
    }
    let t = crate::theme::Tokens::get(ui.ctx());
    if let Some(id) = dots(ui.ctx(), &t) {
        let uv = Rect::from_min_max(Pos2::ZERO, pos2(rect.width() / 22.0, rect.height() / 22.0));
        ui.painter_at(rect).image(id, rect, uv, Color32::WHITE);
    }
}

/// With no document open, double-clicking the empty workspace shows File › Open, as in Photoshop
/// (#5367), and so does the Home screen's blank background. Registered before anything drawn over
/// `area`, so the buttons and links there keep their own clicks; not a keyboard focus stop.
pub(super) fn open_on_double_click(app: &mut PhotocraftApp, ui: &egui::Ui, area: Rect) {
    if app.session.documents().is_empty() && ui.interact(area, ui.scope_id().with("empty-workspace"), Sense::CLICK).double_clicked() {
        let _ = app.open_dialog_file();
    }
}

/// Extra OS windows showing documents (multi-window / multi-monitor).
pub fn extra_windows(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let wins = app.ui.windows.clone();
    for w in wins.into_iter().filter(|w| w.open) {
        let Some(st) = app.session.documents().get(w.document) else { continue };
        let title = format!("{} — window {}", st.doc.name, w.id);
        let vid = egui::ViewportId::from_hash_of(("docwin", w.id));
        let mut builder = crate::os_window::builder().with_title(title).with_inner_size([800.0, 600.0]);
        if let Some([x, y]) = w.position {
            builder = builder.with_position([x, y]);
        }
        let mut view = w.view.clone();
        let mut close = false;
        let mut focused = false;
        ctx.show_viewport_immediate(vid, builder, |ui, class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                close = true;
            }
            focused = ui.ctx().input(|i| i.viewport().focused == Some(true));
            if focused && !w.focused && app.session.active_index() != Some(w.document) {
                app.session.set_active(w.document);
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(crate::theme::Tokens::get(ui.ctx()).canvas)).show(ui, |ui| {
                let rect = ui.available_rect_before_wrap();
                view = canvas_view(app, ui, w.document, rect, view.clone(), focused);
                // Preferences › Zoom Resizes Windows: a zoom wraps the window around the image.
                crate::zoom_window_fit::follow(app, ui.ctx(), class, vid, &w, &mut view, rect);
            });
        });
        if let Some(win) = app.ui.windows.iter_mut().find(|x| x.id == w.id) {
            win.view = view;
            win.focused = focused;
            if close {
                win.open = false;
            }
        }
    }
    app.ui.windows.retain(|w| w.open);
}

/// Window › Arrange tiles: each document shown with its tile in `rect`, or `None` when the
/// active document fills the area.
pub(super) fn docked_document_indices(app: &PhotocraftApp) -> Vec<usize> {
    (0..app.session.documents().len()).filter(|&doc| !crate::view_cmds::document_is_floating(app, doc)).collect()
}

pub(super) fn main_canvas_document(app: &PhotocraftApp) -> Option<usize> {
    let active = app.session.active_index()?;
    if !crate::view_cmds::document_is_floating(app, active) {
        return Some(active);
    }
    docked_document_indices(app).into_iter().next()
}

/// Convert a final tab index in the visible (docked-only) strip to the document list index,
/// keeping hidden floating documents in their existing relative order.
pub(super) fn move_target_after_filter(visible: &[usize], from: usize, to: usize, document_count: usize) -> usize {
    let remaining: Vec<usize> = visible.iter().copied().filter(|&doc| doc != from).collect();
    match remaining.get(to) {
        Some(&doc) if doc > from => doc - 1,
        Some(&doc) => doc,
        None => document_count.saturating_sub(1),
    }
}
