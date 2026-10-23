//! Window › Layer Comps panel (a tab of the History card).
//!
//! Rows: the Last Document State, then each comp with its apply marker, name, warning triangle
//! (layers deleted since capture) and the three apply toggles (visibility, position,
//! appearance). Footer: previous / next comp, update, new, delete. Every action is a
//! `layerComp.*` engine command; the selection is `UiState::layer_comp_selected`.

use egui::{Align2, Rect, Sense, Stroke, pos2, vec2};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;

/// The comp footer and row actions act on: the panel selection if still valid, else the last
/// applied comp.
pub fn target(app: &PhotocraftApp) -> Option<u32> {
    let doc = &app.session.active()?.doc;
    app.ui.layer_comp_selected.filter(|id| doc.comp(*id).is_some()).or(doc.last_applied_comp.filter(|id| doc.comp(*id).is_some()))
}

/// The row's warning triangle: Clear Layer Comp Warning for that comp only (Photoshop keeps
/// Clear All Layer Comp Warnings as a separate command).
fn clear_warning_action(comp: u32) -> (&'static str, Value) {
    ("layerComp.updateWarnings", json!({"clear": true, "comp": comp}))
}

/// The comp whose name is being edited in place, kept in egui's temp memory under its row id.
#[derive(Clone)]
struct RenameComp {
    text: String,
    focused: bool,
}

/// The in-place name field of a comp. Enter, Tab or clicking elsewhere returns the new name to
/// commit; Escape cancels and keeps the old name.
fn rename_field(ui: &mut egui::Ui, id: egui::Id, rect: Rect) -> Option<String> {
    let mut r = ui.data(|d| d.get_temp::<RenameComp>(id))?;
    ui.data_mut(|d| d.remove::<RenameComp>(id));
    let te = ui.put(rect, egui::TextEdit::singleline(&mut r.text).id(id).font(egui::FontId::proportional(12.0)));
    if !r.focused {
        te.request_focus();
        // Like Photoshop, the whole name starts selected so typing replaces it.
        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(r.text.chars().count()));
            state.cursor.set_char_range(Some(all));
            state.store(ui.ctx(), id);
        }
        r.focused = true;
        ui.data_mut(|d| d.insert_temp(id, r));
        return None;
    }
    let (enter, tab, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Tab), i.key_pressed(egui::Key::Escape)));
    if esc {
        None
    } else if enter || tab || te.lost_focus() || !te.has_focus() {
        Some(r.text)
    } else {
        ui.data_mut(|d| d.insert_temp(id, r));
        None
    }
}

fn run(app: &mut PhotocraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

#[derive(Clone, Copy)]
struct DragComp {
    document: photocraft_doc::DocId,
    comp: u32,
}

fn drop_index(from: usize, target: usize, after: bool) -> usize {
    let slot = target + usize::from(after);
    if from < slot { slot - 1 } else { slot }
}

pub fn panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint).size(11.5));
        return;
    };
    let doc = st.doc.clone();
    let selected = target(app);
    let applied = doc.last_applied_comp;
    let max_h = (ui.available_height() - 70.0).clamp(80.0, 320.0);
    let mut action: Option<(&'static str, Value)> = None;
    let mut select: Option<u32> = None;
    let row_h = 26.0;
    egui::ScrollArea::vertical().id_salt("comps-rows").max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
        // Last Document State.
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
        if applied.is_none() {
            ui.painter().rect_filled(rect, 0.0, t.row_selected);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
        }
        let marker = Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(12.0, 12.0));
        ui.painter().rect_stroke(marker, 2.0, Stroke::new(1.0, t.text_faint), egui::StrokeKind::Inside);
        if applied.is_none() {
            crate::icons::paint(ui, marker, "check", 10.0, t.icon);
        }
        ui.painter().text(
            pos2(rect.left() + 30.0, rect.center().y),
            Align2::LEFT_CENTER,
            tl!("Last Document State"),
            egui::FontId::proportional(12.0),
            t.text_dim,
        );
        if resp.clicked() && doc.last_document_state.is_some() && applied.is_some() {
            action = Some(("layerComp.restoreLastDocumentState", json!({})));
        }
        ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));

        if doc.layer_comps.is_empty() {
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(tl!("Capture the visibility, position and style of every layer with + below, then switch between versions."))
                    .color(t.text_faint)
                    .size(11.5),
            );
        }
        for (index, c) in doc.layer_comps.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
            resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::SelectableLabel, true, selected == Some(c.id), &c.name));
            resp.dnd_set_drag_payload(DragComp { document: doc.id, comp: c.id });
            if let Some(source) = resp.dnd_hover_payload::<DragComp>()
                && source.document == doc.id
                && let Some(from) = doc.layer_comps.iter().position(|c| c.id == source.comp)
            {
                let after = ui.input(|i| i.pointer.hover_pos().is_some_and(|p| p.y >= rect.center().y));
                let to = drop_index(from, index, after);
                let y = if after { rect.bottom() } else { rect.top() };
                ui.painter().line_segment([pos2(rect.left(), y), pos2(rect.right(), y)], Stroke::new(2.0, t.accent));
                if resp.dnd_release_payload::<DragComp>().is_some() && from != to {
                    select = Some(source.comp);
                    action = Some(("layerComp.reorder", json!({"comp":source.comp, "index":to})));
                }
            }
            resp.context_menu(|ui| {
                if ui.add_enabled(index > 0, egui::Button::new(tl!("Move Up"))).clicked() {
                    action = Some(("layerComp.reorder", json!({"comp":c.id, "index":index.saturating_sub(1)})));
                    select = Some(c.id);
                    ui.close();
                }
                if ui.add_enabled(index + 1 < doc.layer_comps.len(), egui::Button::new(tl!("Move Down"))).clicked() {
                    action = Some(("layerComp.reorder", json!({"comp":c.id, "index":index + 1})));
                    select = Some(c.id);
                    ui.close();
                }
            });
            if selected == Some(c.id) {
                ui.painter().rect_filled(rect, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.5));
            }
            // Apply marker (click to apply).
            let marker = Rect::from_center_size(pos2(rect.left() + 14.0, rect.center().y), vec2(12.0, 12.0));
            ui.painter().rect_stroke(marker, 2.0, Stroke::new(1.0, t.text_faint), egui::StrokeKind::Inside);
            if applied == Some(c.id) {
                crate::icons::paint(ui, marker, "check", 10.0, t.accent);
            }
            // The three apply toggles, right-aligned.
            let toggles = [
                ("eye", tl!("Visibility"), c.apply_visibility, "visibility"),
                ("move", tl!("Position"), c.apply_position, "position"),
                ("sparkles", tl!("Appearance (Layer Style)"), c.apply_appearance, "appearance"),
            ];
            let mut x = rect.right() - 14.0;
            let mut toggle_hit = false;
            for (icon, tip, on, key) in toggles.iter().rev() {
                let r = Rect::from_center_size(pos2(x, rect.center().y), vec2(20.0, 20.0));
                let hov = ui.rect_contains_pointer(r);
                if hov {
                    ui.painter().rect_filled(r, 3.0, t.hover);
                }
                crate::icons::paint(ui, r, icon, 13.0, if *on { t.icon } else { t.text_faint.gamma_multiply(0.45) });
                if hov && resp.clicked() {
                    toggle_hit = true;
                    action = Some(("layerComp.setOptions", json!({"comp": c.id, *key: !on})));
                }
                if hov {
                    let template = "Apply {option}: {state}";
                    let state = if *on { "on" } else { "off" };
                    resp.clone().on_hover_text(crate::i18n::fmt(tl!(template), &[("option", tip), ("state", tl!(state))]));
                }
                x -= 22.0;
            }
            let missing = c.missing_layers(&doc).len();
            let mut name_x = rect.left() + 30.0;
            if missing > 0 {
                let r = Rect::from_center_size(pos2(name_x + 7.0, rect.center().y), vec2(14.0, 14.0));
                crate::icons::paint(ui, r, "triangle-alert", 12.0, t.warning);
                if ui.rect_contains_pointer(r) {
                    resp.clone().on_hover_text(crate::i18n::trn(
                        crate::i18n::current(),
                        missing as u64,
                        "{n} layer recorded by this comp was deleted. Update the comp, or clear the warning.",
                        "{n} layers recorded by this comp were deleted. Update the comp, or clear the warning.",
                    ));
                    if resp.clicked() {
                        toggle_hit = true;
                        action = Some(clear_warning_action(c.id));
                    }
                }
                name_x += 18.0;
            }
            let galley_w = (x - name_x).max(20.0);
            let name = ui.painter().layout(c.name.clone(), egui::FontId::proportional(12.0), t.text, galley_w);
            let name_rect = Rect::from_min_size(pos2(name_x, rect.center().y - name.size().y / 2.0), name.size());
            // Comp ids restart in every document, so the pending name is keyed by the document too.
            let rename_id = egui::Id::new(("comp-rename", doc.id.0, c.id));
            if ui.data(|d| d.get_temp::<RenameComp>(rename_id)).is_some() {
                let field = Rect::from_min_max(pos2(name_x - 3.0, rect.top() + 3.0), pos2((x + 10.0).max(name_x + 60.0), rect.bottom() - 3.0));
                if let Some(text) = rename_field(ui, rename_id, field) {
                    let text = text.trim();
                    if !text.is_empty() && text != c.name {
                        action = Some(("layerComp.rename", json!({"comp": c.id, "name": text})));
                    }
                }
            } else {
                ui.painter().galley(name_rect.min, name, t.text);
            }
            if !c.comment.is_empty() && resp.hovered() {
                resp.clone().on_hover_text(&c.comment);
            }
            if resp.clicked() && !toggle_hit {
                select = Some(c.id);
                if resp.interact_pointer_pos().is_some_and(|p| p.x < rect.left() + 24.0) {
                    action = Some(("layerComp.apply", json!({"comp": c.id})));
                }
            }
            if resp.double_clicked() && !toggle_hit {
                // Photoshop renames in place when the name itself is double-clicked.
                if resp.interact_pointer_pos().is_some_and(|p| name_rect.expand2(vec2(4.0, 13.0)).contains(p)) {
                    ui.data_mut(|d| d.insert_temp(rename_id, RenameComp { text: c.name.clone(), focused: false }));
                } else {
                    action = Some(("layerComp.apply", json!({"comp": c.id})));
                }
            }
            ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator.gamma_multiply(0.5)));
        }
    });
    // Footer: apply previous, apply next | update, new, delete (Photoshop's order).
    ui.add_space(4.0);
    let has = !doc.layer_comps.is_empty();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if crate::icons::button(ui, "chevrons-left", 24.0, false, tl!("Apply previous selected layer comp")).clicked() && has {
            action = Some(("layerComp.previous", json!({})));
        }
        if crate::icons::button(ui, "chevrons-right", 24.0, false, tl!("Apply next selected layer comp")).clicked() && has {
            action = Some(("layerComp.next", json!({})));
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "trash", 24.0, false, tl!("Delete layer comp")).clicked()
                && let Some(id) = selected
            {
                action = Some(("layerComp.delete", json!({"comp": id})));
            }
            if crate::icons::button(ui, "file-plus", 24.0, false, tl!("Create new layer comp")).clicked() {
                action = Some(("layerComp.new", json!({})));
            }
            if crate::icons::button(ui, "rotate-cw", 24.0, false, tl!("Update layer comp (visibility, position and appearance)")).clicked()
                && let Some(id) = selected
            {
                action = Some(("layerComp.update", json!({"comp": id})));
            }
        });
    });
    if let Some(id) = select {
        app.ui.layer_comp_selected = Some(id);
    }
    if let Some((cmd, p)) = action {
        let created = cmd == "layerComp.new";
        run(app, cmd, p);
        if created {
            app.ui.layer_comp_selected = app.session.active().and_then(|d| d.doc.last_applied_comp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;

    #[test]
    fn dragging_a_comp_row_reorders_without_applying_it() {
        use egui_kittest::{Harness, kittest::Queryable};
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":16,"height":16})).unwrap();
        for name in ["First", "Second", "Third"] {
            app.run("layerComp.new", json!({"name":name})).unwrap();
        }
        let applied = app.session.active().unwrap().doc.last_applied_comp;
        let first = app.session.active().unwrap().doc.layer_comps[0].id;
        let mut h = Harness::builder().with_size(vec2(360.0, 300.0)).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let from = h.get_by_label("First").rect().center();
        let row = h.get_by_label("Third").rect();
        let to = pos2(row.center().x, row.bottom() - 2.0);
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerMoved(from + vec2(10.0, 0.0)));
        h.run_steps(2);
        h.event(egui::Event::PointerMoved(to));
        h.run_steps(2);
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
        let app = h.state();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(doc.layer_comps.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Second", "Third", "First"]);
        assert_eq!(doc.last_applied_comp, applied);
        assert_eq!(app.ui.layer_comp_selected, Some(first));
    }

    #[test]
    fn comp_drop_slots_account_for_the_removed_source_row() {
        assert_eq!(drop_index(0, 2, false), 1);
        assert_eq!(drop_index(0, 2, true), 2);
        assert_eq!(drop_index(2, 0, false), 0);
        assert_eq!(drop_index(2, 0, true), 1);
        assert_eq!(drop_index(1, 1, false), 1);
        assert_eq!(drop_index(1, 1, true), 1);
    }

    #[test]
    fn warning_icon_clears_only_its_own_comp() {
        // Regression for #4197.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
        let layer = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let c1 = s.execute("layerComp.new", json!({"name": "One"})).unwrap()["comp"].as_u64().unwrap() as u32;
        let c2 = s.execute("layerComp.new", json!({"name": "Two"})).unwrap()["comp"].as_u64().unwrap() as u32;
        s.execute("layer.delete", json!({"layer": layer})).unwrap();
        let (id, p) = clear_warning_action(c1);
        assert_eq!(p["comp"], c1);
        s.execute(id, p).unwrap();
        let doc = &s.active().unwrap().doc;
        assert!(doc.comp(c1).unwrap().missing_layers(doc).is_empty());
        assert_eq!(doc.comp(c2).unwrap().missing_layers(doc).iter().map(|l| l.0).collect::<Vec<_>>(), vec![layer]);
    }

    #[test]
    fn double_clicking_a_comp_name_renames_it_in_place() {
        // #6496.
        use egui_kittest::{
            Harness,
            kittest::{NodeT, Queryable},
        };
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let first = app.run("layerComp.new", json!({"name": "Comp number one"})).unwrap()["comp"].as_u64().unwrap() as u32;
        let second = app.run("layerComp.new", json!({"name": "Two"})).unwrap()["comp"].as_u64().unwrap() as u32;
        // 60 fps steps, so two clicks a frame apart count as a double-click.
        let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        // The footer starts at the panel's left edge; the first comp is the second 26 px row. The
        // name starts at left + 30 and is as wide as its text, so the click stays near its start
        // to hit the short "Hero" too.
        let left = h.get_by_label("Apply previous selected layer comp").rect().left();
        let (name, rest) = (pos2(left + 35.0, left + 39.0), pos2(left + 190.0, left + 39.0));
        let double_click = |h: &mut Harness<'_, PhotocraftApp>, pos: egui::Pos2| {
            h.event(egui::Event::PointerMoved(pos));
            // Wait out the double-click window of the previous attempt.
            h.run_steps(30);
            for pressed in [true, false, true, false] {
                h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.step();
            }
            h.run_steps(3);
        };
        let type_name = |h: &mut Harness<'_, PhotocraftApp>, text: &str, key: egui::Key| {
            double_click(h, name);
            h.event(egui::Event::Text(text.into()));
            h.run_steps(1);
            let fields: Vec<_> = h.query_all_by_role(egui::accesskit::Role::TextInput).map(|n| n.accesskit_node().value()).collect();
            assert_eq!(fields, vec![Some(text.to_string())], "the name field opened and holds {text:?}");
            h.key_press(key);
            h.run_steps(3);
        };
        let comp_name = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.comp(first).unwrap().name.clone();
        let applied = |h: &Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().doc.last_applied_comp;
        // Whitespace keeps the old name.
        type_name(&mut h, "  ", egui::Key::Enter);
        assert_eq!(comp_name(&h), "Comp number one");
        type_name(&mut h, "Hero", egui::Key::Enter);
        assert_eq!(comp_name(&h), "Hero");
        assert_eq!(applied(&h), Some(second), "renaming must not apply the comp");
        type_name(&mut h, "Lost", egui::Key::Escape);
        assert_eq!(comp_name(&h), "Hero", "Escape cancels");
        assert_eq!(applied(&h), Some(second), "Escape applies nothing");
        // Double-clicking the rest of the row still applies the comp.
        double_click(&mut h, rest);
        assert_eq!(applied(&h), Some(first));
        assert_eq!(comp_name(&h), "Hero");
    }

    #[test]
    fn a_rename_left_open_does_not_carry_over_to_another_document() {
        // Comp ids restart in every document, so the pending name must stay with its own document.
        use egui_kittest::{
            Harness,
            kittest::{NodeT, Queryable},
        };
        let mut app = PhotocraftApp::new(Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let a = app.run("layerComp.new", json!({"name": "Alpha comp"})).unwrap()["comp"].as_u64().unwrap() as u32;
        app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
        let b = app.run("layerComp.new", json!({"name": "Beta comp"})).unwrap()["comp"].as_u64().unwrap() as u32;
        assert_eq!(a, b, "the two documents' comps share an id");
        app.session.set_active(0);
        let mut h = Harness::builder().with_size(vec2(320.0, 260.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| panel(app, ui), app);
        h.run_steps(3);
        let left = h.get_by_label("Apply previous selected layer comp").rect().left();
        let name = pos2(left + 35.0, left + 39.0);
        h.event(egui::Event::PointerMoved(name));
        h.run_steps(3);
        for pressed in [true, false, true, false] {
            h.event(egui::Event::PointerButton { pos: name, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
        h.event(egui::Event::Text("Leak".into()));
        h.run_steps(1);
        let fields: Vec<_> = h.query_all_by_role(egui::accesskit::Role::TextInput).map(|n| n.accesskit_node().value()).collect();
        assert_eq!(fields, vec![Some("Leak".to_string())], "the name field opened in the first document");
        h.state_mut().session.set_active(1);
        h.run_steps(3);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().doc.comp(b).unwrap().name, "Beta comp");
    }
}
