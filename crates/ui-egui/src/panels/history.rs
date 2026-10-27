use super::layer_drag::dropped_on;
use super::*;

pub(super) fn history(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let entries = st.history.entries();
    let current = entries.len() - 1;
    let redo: Vec<String> = st.history.redo_labels().map(str::to_string).collect();
    let doc_id = st.doc.id;
    let source = photocraft_engine::history_source_cmds::source(st);
    let selected_snap = selected_snapshot(ui.ctx(), st);
    let spot = (doc_id, history_spot(st));
    let ctx = ui.ctx().clone();
    // A row dragged onto the trash (#5324); a drag that never saw its release must not outlive it.
    if !ctx.input(|i| i.pointer.primary_down() || i.pointer.any_released()) {
        ctx.data_mut(|d| d.remove::<HistoryDrag>(history_drag_key()));
    }
    let drag = ctx.data(|d| d.get_temp::<HistoryDrag>(history_drag_key()));
    let mut snap_cmd = None;
    let mut target: Option<usize> = None;
    let mut set_source: Option<usize> = None;
    let all = entries.iter().map(|e| (e.clone(), false)).chain(redo.iter().map(|e| (e.clone(), true)));
    // The dock gives History a fixed height: the rows scroll above the footer.
    let footer = if t.pro { widgets::footer_height(ui) + ui.spacing().item_spacing.y } else { 0.0 };
    let max_h = (ui.available_height() - footer).max(40.0);
    // Follow the current state when the history moves (a new step, Undo, a state row) or another
    // document comes up (`spot`), scrolling only if its row is hidden (#5909). Other changes (a
    // layer click, a channel eye) and a selected snapshot leave the list where the user put it.
    let follow_id = egui::Id::unique("history-follow");
    let moved = ui.ctx().data_mut(|d| {
        let moved = d.get_temp::<(photocraft_doc::DocId, HistorySpot)>(follow_id).as_ref() != Some(&spot);
        d.insert_temp(follow_id, spot);
        moved
    });
    let follow = moved && selected_snap.is_none();
    egui::ScrollArea::vertical().id_salt("history-rows").max_height(max_h).auto_shrink([false, false]).show(ui, |ui| {
        // Snapshots head the list and scroll with the states, so many of them never push the
        // footer out of a short dock.
        snap_cmd = snapshot_rows(app, ui, selected_snap);
        let mut current_rect = None;
        for (i, (e, is_redo)) in all.enumerate() {
            // The current state shows selected unless a snapshot is (Photoshop highlights one row).
            let selected = i == current && selected_snap.is_none();
            // Past states can be dragged onto the trash; the oldest one can't be deleted.
            let sense = if is_redo || i == 0 { Sense::click() } else { Sense::click_and_drag() };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), sense);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &e));
            if resp.drag_started() {
                ui.ctx().data_mut(|d| d.insert_temp(history_drag_key(), HistoryDrag::State(i)));
            }
            if i == current {
                current_rect = Some(rect);
            }
            if selected {
                ui.painter().rect_filled(rect, if t.pro { 0.0 } else { t.radius_sm }, if t.pro { t.row_selected } else { t.accent_soft });
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
            }
            let icon = match e.as_str() {
                s if s.contains("Brush") || s.contains("Eraser") => "brush",
                // The Layers panel's controls (layer.setProps).
                "Opacity Change" | "Fill Opacity Change" | "Blending Change" => "layers",
                s if s.contains("Layer") => "layers",
                s if s.contains("Select") || s.contains("Marquee") || s.contains("Deselect") || s.contains("Lasso") => "square-dashed",
                s if s.contains("Fill") => "paint-bucket",
                "Open" => "image",
                _ => "sliders-horizontal",
            };
            let color = if is_redo { t.text_faint } else { t.text };
            icons::paint(
                ui,
                Rect::from_center_size(pos2(rect.left() + 42.0, rect.center().y), vec2(16.0, 16.0)),
                icon,
                12.0,
                if is_redo { t.text_faint } else { t.icon },
            );
            ui.painter().text(pos2(rect.left() + 58.0, rect.center().y), Align2::LEFT_CENTER, &e, egui::FontId::proportional(12.0), color);
            if history_source_column(ui, rect, egui::Id::unique(("history-source", i)), source == Some(HistorySource::State(i))) {
                set_source = Some(i);
            }
            if resp.clicked() && i != current {
                target = Some(i);
            } else if resp.clicked() && selected_snap.is_some() {
                // The current state is already the document: clicking it only selects it.
                ctx.data_mut(|d| d.remove::<SelectedSnapshot>(selected_snapshot_key(doc_id)));
            }
        }
        if follow && let Some(rect) = current_rect {
            // No alignment: a row already in view stays put. No animation, as the Layers panel reveal.
            ui.scroll_to_rect_animation(rect, None, egui::style::ScrollAnimation::none());
        }
    });
    if let Some(i) = set_source
        && let Err(e) = app.run("history.setSource", json!({"state": i}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if let Some(drag) = drag
        && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
    {
        let label = match drag {
            HistoryDrag::Snapshot(id) => app.session.active().and_then(|st| st.snapshots.iter().find(|s| s.id == id)).map(|s| s.name.clone()),
            HistoryDrag::State(i) => entries.get(i).cloned(),
        };
        crate::layer_transfer::ghost(&ctx, p, &label.unwrap_or_default());
    }
    let mut footer_cmd: Option<(&str, Value)> = None;
    let mut snapshot_dialog = false;
    if t.pro {
        // With a snapshot selected the trash deletes it, as in Photoshop (#5324).
        let can_delete = selected_snap.is_some() || app.session.is_enabled("history.deleteState");
        let can_new = app.session.is_enabled("history.newDocument");
        widgets::panel_footer(ui, |ui| {
            let trash = ui.add_enabled_ui(can_delete, |ui| icons::button(ui, "trash", 26.0, false, tl!("Delete current state"))).inner;
            if trash.clicked() {
                footer_cmd = Some(match selected_snap {
                    Some(id) => ("history.deleteSnapshot", json!({"snapshot": id})),
                    None => ("history.deleteState", json!({})),
                });
            }
            // A snapshot or a past state dragged onto the trash is deleted (#5324).
            if let Some(drag) = drag
                && dropped_on(ui, &trash)
            {
                footer_cmd = Some(match drag {
                    HistoryDrag::Snapshot(id) => ("history.deleteSnapshot", json!({"snapshot": id})),
                    HistoryDrag::State(i) => ("history.deleteState", json!({"state": i})),
                });
            }
            let can_snap = app.session.is_enabled("history.newSnapshot");
            // ⌥-click opens New Snapshot (name, From) instead, as in Photoshop.
            if ui.add_enabled_ui(can_snap, |ui| icons::button(ui, "scan", 26.0, false, tl!("Create new snapshot"))).inner.clicked() {
                if ui.input(|i| i.modifiers.alt) {
                    snapshot_dialog = true;
                } else {
                    footer_cmd = Some(("history.newSnapshot", json!({})));
                }
            }
            if ui.add_enabled_ui(can_new, |ui| icons::button(ui, "file-plus", 26.0, false, tl!("Create new document from current state"))).inner.clicked() {
                footer_cmd = Some(("history.newDocument", json!({})));
            }
        });
    }
    if ctx.input(|i| i.pointer.any_released()) {
        ctx.data_mut(|d| d.remove::<HistoryDrag>(history_drag_key()));
    }
    // An open Free Transform owns Undo (transform_tool::intercept): stepping the document's history under
    // its box would leave it transforming pixels that changed.
    if snapshot_dialog {
        crate::snapshot_dialog::open(app);
    }
    // Selecting a snapshot changes the document, so it waits for an open transform like the rows.
    let snap_cmd = snap_cmd.filter(|(cmd, _)| *cmd != "history.selectSnapshot" || app.ui.transform.is_none());
    if let Some((cmd, p)) = footer_cmd.filter(|_| app.ui.transform.is_none()).or(snap_cmd) {
        let picked = (cmd == "history.selectSnapshot").then(|| p.get("snapshot").and_then(Value::as_u64)).flatten();
        match app.run(cmd, p) {
            Err(e) => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            // The snapshot stays selected while the document is the state the click made.
            Ok(_) => {
                if let (Some(id), Some(st)) = (picked, app.session.active()) {
                    select_snapshot(&ctx, st, id);
                }
            }
        }
    }
    // One command per click (#2966): stepping with Undo/Redo once per state made the canvas
    // recomposite the whole document.
    if let Some(state) = target.filter(|_| app.ui.transform.is_none())
        && let Err(e) = app.run("history.selectState", json!({ "state": state }))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// The History panel's source column at the left of a state or snapshot row: the History Brush
/// icon on the source; a click makes the row the source (`history.setSource`). It sits above the
/// row, so the click doesn't also step to the state or snapshot.
fn history_source_column(ui: &mut egui::Ui, row: Rect, salt: egui::Id, is_source: bool) -> bool {
    let t = Tokens::get(ui.ctx());
    let cell = Rect::from_min_size(row.min, vec2(24.0, row.height()));
    let resp = ui.interact(cell, ui.scope_id().with(salt), Sense::click()).on_hover_text(tl!("Sets the source for the history brush"));
    if is_source {
        icons::paint(ui, cell.shrink(4.0), icons::tool_icon(Tool::HistoryBrush), 12.0, t.icon);
    } else if resp.hovered() {
        icons::paint(ui, cell.shrink(4.0), icons::tool_icon(Tool::HistoryBrush), 12.0, t.text_faint);
    }
    resp.clicked()
}

/// A History panel row being dragged, for a drop on the trash (#5324).
#[derive(Clone, Copy, Debug)]
enum HistoryDrag {
    Snapshot(u64),
    State(usize),
}

fn history_drag_key() -> egui::Id {
    egui::Id::unique("history-drag")
}

/// The snapshot the History panel shows selected (#5324), and where the history stood after its
/// click: the number of past states, the current state's name, the number of redo states and the
/// newest past state. That last one is a new document on every step, also at the state limit,
/// where the counts and a repeated name stay the same.
type SelectedSnapshot = (u64, HistorySpot);

type HistorySpot = (usize, Option<String>, usize, Option<usize>);

fn history_spot(st: &photocraft_engine::DocState) -> HistorySpot {
    let newest = st.history.past_len().checked_sub(1).and_then(|i| st.history.state(i)).map(|d| std::sync::Arc::as_ptr(&d) as usize);
    (st.history.past_len(), st.history.entries().pop(), st.history.redo_labels().count(), newest)
}

fn selected_snapshot_key(doc: photocraft_doc::DocId) -> egui::Id {
    egui::Id::unique(("history-snapshot-selected", doc))
}

/// Select snapshot `id` at the history's current state, as a click on its row does.
pub(crate) fn select_snapshot(ctx: &egui::Context, st: &photocraft_engine::DocState, id: u64) {
    ctx.data_mut(|d| d.insert_temp::<SelectedSnapshot>(selected_snapshot_key(st.doc.id), (id, history_spot(st))));
}

/// The selected snapshot: the one last clicked, while it still exists and the history is still at
/// the state that click made. Any other step (an edit, Undo, a state row) deselects it, as in
/// Photoshop.
pub(crate) fn selected_snapshot(ctx: &egui::Context, st: &photocraft_engine::DocState) -> Option<u64> {
    let key = selected_snapshot_key(st.doc.id);
    let (id, spot) = ctx.data(|d| d.get_temp::<SelectedSnapshot>(key))?;
    let live = spot == history_spot(st) && st.snapshots.iter().any(|s| s.id == id);
    if !live {
        ctx.data_mut(|d| d.remove::<SelectedSnapshot>(key));
    }
    live.then_some(id)
}

/// The History panel's snapshot rows (#2966), under the opening row, each with its thumbnail and
/// name: a click returns to the snapshot (a new state) and selects it, a double-click or
/// right-click › Rename… edits the name in place (Enter or a click elsewhere keeps it, Escape
/// cancels), right-click › Delete removes it, and a drag onto the trash too (#5324). Returns the
/// command a row asks for.
fn snapshot_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, selected: Option<u64>) -> Option<(&'static str, Value)> {
    let t = Tokens::get(ui.ctx());
    let st = app.session.active()?;
    let (doc_id, snaps) = (st.doc.id, st.snapshots.clone());
    let source = photocraft_engine::history_source_cmds::source(st);
    let live: Vec<u64> = snaps.iter().map(|s| s.id).collect();
    // Per document: snapshot ids restart in each, and a tab switch must not carry the edit over.
    let rename_id = egui::Id::unique(("history-snapshot-rename", doc_id));
    let mut renaming: Option<(u64, String)> = ui.data(|d| d.get_temp(rename_id)).flatten();
    let mut cmd = None;
    let ctx = ui.ctx().clone();
    for snap in &snaps {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click_and_drag());
        let is_selected = selected == Some(snap.id);
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, is_selected, &snap.name));
        if resp.drag_started() {
            ctx.data_mut(|d| d.insert_temp(history_drag_key(), HistoryDrag::Snapshot(snap.id)));
        }
        if is_selected {
            ui.painter().rect_filled(rect, if t.pro { 0.0 } else { t.radius_sm }, if t.pro { t.row_selected } else { t.accent_soft });
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
        }
        let thumb = Rect::from_min_size(pos2(rect.left() + 28.0, rect.center().y - 14.0), vec2(28.0, 28.0));
        widgets::checker(ui.painter(), thumb, 4.0);
        let tex = app.snapshot_thumb(&ctx, doc_id, snap, &live);
        ui.painter().image(tex, thumb, Rect::from_min_max(egui::Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        ui.painter().rect_stroke(thumb, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        match renaming.as_mut().filter(|(id, _)| *id == snap.id) {
            Some((_, text)) => {
                let field = Rect::from_min_max(pos2(thumb.right() + 6.0, rect.center().y - 10.0), pos2(rect.right() - 6.0, rect.center().y + 10.0));
                let r = ui.put(field, egui::TextEdit::singleline(text)).accessible_name(tl!("Name"));
                if r.lost_focus() {
                    if !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cmd = Some(("history.renameSnapshot", json!({"snapshot": snap.id, "name": text.clone()})));
                    }
                    renaming = None;
                } else if !r.has_focus() {
                    // Starting to rename: focus the field with the whole name selected, so typing
                    // replaces it (Photoshop).
                    r.request_focus();
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), r.id) {
                        let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(text.chars().count()));
                        state.cursor.set_char_range(Some(all));
                        state.store(ui.ctx(), r.id);
                    }
                }
            }
            None => {
                ui.painter().text(pos2(thumb.right() + 8.0, rect.center().y), Align2::LEFT_CENTER, &snap.name, egui::FontId::proportional(12.0), t.text);
            }
        }
        if history_source_column(ui, rect, egui::Id::unique(("history-snapshot-source", snap.id)), source == Some(HistorySource::Snapshot(snap.id))) {
            cmd = Some(("history.setSource", json!({"snapshot": snap.id})));
        } else if resp.double_clicked() {
            renaming = Some((snap.id, snap.name.clone()));
        } else if resp.clicked() {
            cmd = Some(("history.selectSnapshot", json!({"snapshot": snap.id})));
        }
        resp.context_menu(|ui| {
            if ui.button(tl!("Rename…")).clicked() {
                renaming = Some((snap.id, snap.name.clone()));
                ui.close();
            }
            if ui.button(tl!("Delete")).clicked() {
                cmd = Some(("history.deleteSnapshot", json!({"snapshot": snap.id})));
                ui.close();
            }
        });
    }
    if !snaps.is_empty() {
        let y = ui.cursor().top();
        ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, t.separator));
    }
    ui.data_mut(|d| d.insert_temp(rename_id, renaming));
    cmd
}
