use super::layer_drag::{
    FxDrag, MaskDrag, dropped_on, effect_rows, footer_drop, fx_drag_feedback, fx_drag_key, fx_trash_drop, layer_drag_edge_scroll, layer_drop_zone,
    mask_drag_key,
};
use super::layer_row::{RowSel, layer_row};
use super::options_bar::{LAYER_PCT_W, body_text_width, pct_action, scrub_pct_label};
use super::*;

// ----------------------------------------------------------------------------- layers

/// The simple themes' single lock button. On the Background it converts the layer to a normal
/// one, as clicking the Background's lock does in Photoshop. Elsewhere it clears every lock when
/// any is set, and locks all otherwise. Toggling only "lock all" would leave the Background's
/// transparency lock (still showing as locked), and a second click would lock its pixels,
/// shutting out the Eraser and every other paint tool (#76).
/// The Basic layout's lock toggle: the active layer's state decides, and every target follows it
/// (`targets`: `{"layer": id}` or `{"layers": [ids]}`).
pub(super) fn simple_lock_toggle(background: bool, l: &Layer, targets: &Value) -> (String, Value) {
    if background {
        return ("layer.new.layerFromBackground".into(), json!({}));
    }
    let locks =
        if l.locks.any() { json!({"transparency": false, "pixels": false, "position": false, "artboard": false, "all": false}) } else { json!({"all": true}) };
    let mut p = targets.clone();
    p["locks"] = locks;
    ("layer.setProps".into(), p)
}

/// The Layers panel's blend modes: Photoshop's, plus the layer's own Paint.NET mode when it has one
/// (an imported `.pdn`), so the menu matches Photoshop's for every other layer.
fn blend_options(groups: bool, current: BlendMode) -> Vec<(BlendMode, &'static str)> {
    let own = Some(current).filter(|m| !m.has_psd_equivalent());
    std::iter::once(BlendMode::PassThrough).filter(|_| groups).chain(BlendMode::LAYER_MODES).chain(own).map(|m| (m, m.label())).collect()
}

/// Fit blend mode + Opacity into the available Layers-panel width (#2309).
/// Preserve the normal translated label when there is room; on a narrow dock,
/// keep the field interactive and give the dropdown the remaining width.
pub(super) fn opacity_row_layout(available: f32, label_width: f32, gap: f32) -> (f32, bool) {
    let full_right = label_width + LAYER_PCT_W + 2.0 * gap + 16.0;
    // The label shows while the blend dropdown keeps a usable width (at the default dock width
    // with an English label it does; a long translated label on a narrow dock gives way).
    let show_label = available >= full_right + 80.0;
    let reserved = LAYER_PCT_W + gap + if show_label { label_width + gap + 16.0 } else { 0.0 };
    ((available - reserved).max(40.0), show_label)
}

/// Blend mode, Opacity, the locks and Fill with no layer selected: Photoshop keeps them in place,
/// greyed out at Normal, 100% and unlocked, instead of removing them (#8880). Same layout as the
/// active layer's rows in `layers`.
fn inactive_blend_rows(ui: &mut egui::Ui, pro: bool) {
    ui.add_enabled_ui(false, |ui| {
        ui.horizontal(|ui| {
            let opacity_label = if pro { tl!("Opacity:") } else { tl!("Opacity") };
            let (blend_width, show_opacity_label) = opacity_row_layout(ui.available_width(), body_text_width(ui, opacity_label), ui.spacing().item_spacing.x);
            let mut m = BlendMode::Normal;
            let options = blend_options(false, m);
            widgets::dropdown_wheel_hovered(ui, "blend", tl!("Blend Mode"), &mut m, &options, blend_width);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut o = 100.0;
                widgets::popup_value_field(ui, opacity_label, &mut o, 0.0..=100.0, "%", LAYER_PCT_W);
                if show_opacity_label {
                    scrub_pct_label(ui, opacity_label, &mut o, ui.available_width());
                }
            });
        });
        ui.add_space((theme::ROW_GAP - ui.spacing().item_spacing.y).max(0.0));
        ui.horizontal(|ui| {
            let lock_label = if pro { tl!("Lock:") } else { tl!("Lock") };
            let lang = crate::i18n::current();
            let fill_label = if pro { crate::i18n::tr_ctx(lang, "layers", "Fill:") } else { crate::i18n::tr_ctx(lang, "layers", "Fill") };
            let gap = ui.spacing().item_spacing.x;
            let icons_w = if pro { 5.0 * 20.0 + gap } else { 22.0 + gap };
            let fits = body_text_width(ui, lock_label) + icons_w + body_text_width(ui, fill_label) + LAYER_PCT_W + 2.0 * gap <= ui.available_width();
            if fits {
                label(ui, lock_label);
            }
            if pro {
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    for (icon, tip) in [
                        ("grid-3x3", tl!("Lock transparent pixels")),
                        ("brush", tl!("Lock image pixels")),
                        ("move", tl!("Lock position")),
                        ("scan", tl!("Prevent auto-nesting in and out of Artboards and Frames")),
                        ("lock", tl!("Lock all")),
                    ] {
                        icons::button(ui, icon, 20.0, false, tip);
                    }
                });
            } else {
                icons::button(ui, "lock-open", 22.0, false, tl!("Lock layer"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut f = 100.0;
                widgets::popup_value_field(ui, fill_label, &mut f, 0.0..=100.0, "%", LAYER_PCT_W);
                scrub_pct_label(ui, fill_label, &mut f, ui.available_width());
            });
        });
        ui.add_space(2.0);
    });
}

pub(super) fn layers(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    // A new active layer opens its parent groups and is scrolled into view (#152).
    let reveal = crate::layer_reveal::track(app, ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let doc = st.doc.clone();
    let active = st.active_layer;
    let active_layer = active.and_then(|id| doc.layer(id));
    let selection = st.selected_layers();
    let isolated = st.isolated_layers.clone();
    let mut actions: Vec<(String, Value)> = Vec::new();

    let t = Tokens::get(ui.ctx());
    app.reorder_preview_hit = false;
    if !ui.input(|i| i.pointer.primary_down()) || !app.ui.layers_panel_options.preview_on_canvas_when_reordering_layers {
        app.reorder_preview = None;
    }
    if t.pro && app.ui.layers_panel_options.show_filters {
        crate::layer_filter_ui::bar(app, ui);
        ui.add_space(4.0);
    }
    if let Some(l) = active_layer {
        // With several layers selected, blend mode, Opacity, Fill and the locks change every one
        // of them, as in Photoshop; the controls show the active layer's values.
        // The Background stays out of a several-layer change until Photoshop's behaviour is
        // confirmed (#5856): Opacity, Fill or a blend mode would turn it into Layer 0.
        let ids: Vec<LayerId> = if selection.len() > 1 && selection.contains(&l.id) {
            selection.iter().copied().filter(|id| *id == l.id || !doc.layer(*id).is_some_and(|x| crate::doc_props_ui::is_background(&doc, x))).collect()
        } else {
            vec![l.id]
        };
        let targets = if ids.len() > 1 { json!({"layers": ids.iter().map(|id| id.0).collect::<Vec<_>>()}) } else { json!({"layer": l.id.0}) };
        // Photoshop greys blend mode, Opacity and Fill for the Background layer. (With the
        // Background among several selected, what Photoshop does awaits a check: #5856.)
        let bg = crate::doc_props_ui::is_background(&doc, l);
        // Pass Through is a group's mode: offered only when every target is a group.
        let all_groups = ids.iter().all(|id| doc.layer(*id).is_some_and(|x| x.is_group()));
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!bg, |ui| {
                let mut m = l.blend;
                // Reserve space for the numeric field first. On a narrow dock, hide the
                // redundant visible Opacity label rather than force the dropdown beyond
                // the panel's right edge (#2309). The numeric field keeps its name.
                let opacity_label = if t.pro { tl!("Opacity:") } else { tl!("Opacity") };
                let (blend_width, show_opacity_label) =
                    opacity_row_layout(ui.available_width(), body_text_width(ui, opacity_label), ui.spacing().item_spacing.x);
                let (chosen, hovered) =
                    widgets::dropdown_wheel_hovered(ui, "blend", tl!("Blend Mode"), &mut m, &blend_options(all_groups, l.blend), blend_width);
                // One step per choice: a click, an arrow key or each wheel notch (#1747).
                for m in &chosen {
                    let mut p = targets.clone();
                    p["blend"] = json!(m.label());
                    actions.push(("layer.setProps".into(), p));
                }
                // Hovering a mode previews it on the canvas (#970).
                crate::blend_preview::hover(app, &ids, hovered.filter(|_| chosen.is_empty()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut o = l.opacity * 100.0;
                    let field = widgets::popup_value_field(ui, opacity_label, &mut o, 0.0..=100.0, "%", LAYER_PCT_W);
                    // On a narrow dock the (scrubby) label yields its room to the controls (#2309).
                    let scrub = if show_opacity_label {
                        scrub_pct_label(ui, opacity_label, &mut o, ui.available_width())
                    } else {
                        widgets::PopupFieldResponse::default()
                    };
                    if field.changed || scrub.changed {
                        actions.push(pct_action(&targets, "opacity", o, scrub.drag.or(field.drag)));
                    }
                });
            });
        });
        // Docks zero the item spacing: keep the Opacity and Fill fields apart (#155).
        ui.add_space((theme::ROW_GAP - ui.spacing().item_spacing.y).max(0.0));
        ui.horizontal(|ui| {
            let lock_label = if t.pro { tl!("Lock:") } else { tl!("Lock") };
            // Layer fill opacity (≠ Edit › Fill / path fill → Tô).
            let lang = crate::i18n::current();
            let fill_label = if t.pro { crate::i18n::tr_ctx(lang, "layers", "Fill:") } else { crate::i18n::tr_ctx(lang, "layers", "Fill") };
            // In a narrow panel with long translated labels, drop the "Lock:" text (the icons keep
            // their tooltips) rather than let the Fill label run over the lock icons.
            let gap = ui.spacing().item_spacing.x;
            let icons_w = if t.pro { 5.0 * 20.0 + gap } else { 22.0 + gap };
            let fits = body_text_width(ui, lock_label) + icons_w + body_text_width(ui, fill_label) + LAYER_PCT_W + 2.0 * gap <= ui.available_width();
            if fits {
                label(ui, lock_label);
            }
            if t.pro {
                // The lock icons touch, but only they: the Fill label keeps the same gap before its
                // field as Opacity's, so both stack in one right-aligned column (#2085).
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let lk = l.locks;
                    for (key, icon, tip, on) in [
                        ("transparency", "grid-3x3", tl!("Lock transparent pixels"), lk.transparency),
                        ("pixels", "brush", tl!("Lock image pixels"), lk.pixels),
                        ("position", "move", tl!("Lock position"), lk.position),
                        ("artboard", "scan", tl!("Prevent auto-nesting in and out of Artboards and Frames"), lk.artboard),
                        ("all", "lock", tl!("Lock all"), lk.all),
                    ] {
                        // Lock Image Pixels includes Lock Transparent Pixels, which waits for it (#4892).
                        let enabled = key != "transparency" || !lk.pixels;
                        if ui.add_enabled_ui(enabled, |ui| icons::button(ui, icon, 20.0, on, tip)).inner.clicked() {
                            let mut p = targets.clone();
                            p["locks"] = json!({ key: !on });
                            actions.push(("layer.setProps".into(), p));
                        }
                    }
                });
            }
            if !t.pro {
                let locked = l.locks.any();
                if icons::button(ui, if locked { "lock" } else { "lock-open" }, 22.0, locked, tl!("Lock layer")).clicked() {
                    actions.push(simple_lock_toggle(crate::doc_props_ui::is_background(&doc, l), l, &targets));
                }
            }
            ui.add_enabled_ui(!bg, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut f = l.fill_opacity * 100.0;
                    let field = widgets::popup_value_field(ui, fill_label, &mut f, 0.0..=100.0, "%", LAYER_PCT_W);
                    let scrub = scrub_pct_label(ui, fill_label, &mut f, ui.available_width());
                    if field.changed || scrub.changed {
                        actions.push(pct_action(&targets, "fill", f, scrub.drag.or(field.drag)));
                    }
                })
            });
        });
        ui.add_space(2.0);
    } else {
        inactive_blend_rows(ui, t.pro);
    }

    // Top of the stack first, groups above their contents, closed groups' contents hidden (#126).
    let rows = crate::layer_tree_ui::panel_rows(app, &doc);
    let ctx = ui.ctx().clone();
    let footer = widgets::footer_height(ui) + ui.spacing().item_spacing.y;
    let fill = ui.available_height() > footer + 60.0;
    let rows_h = if fill { ui.available_height() - footer } else { f32::INFINITY };
    egui::ScrollArea::vertical()
        // Keep each document's position independent, even when tab indices change (#5522).
        .id_salt(("layer-rows", doc.id.0))
        .max_height(rows_h)
        .min_scrolled_height(if fill { rows_h } else { 0.0 })
        .auto_shrink([false, !fill])
        .show(ui, |ui| {
            // The drag keys are set only by actual layer-row and fx drags, not clicks or
            // ordinary scrolling. The ScrollArea applies this to its own content.
            // An fx drag that never saw its release (the panel was hidden) must not outlive the button.
            if !ctx.input(|i| i.pointer.primary_down() || i.pointer.any_released()) {
                ctx.data_mut(|d| d.remove::<FxDrag>(fx_drag_key()));
                ctx.data_mut(|d| d.remove::<MaskDrag>(mask_drag_key()));
                ctx.data_mut(|d| d.remove::<crate::smart_ui::FilterDrag>(crate::smart_ui::drag_key()));
            }
            let held = ctx.data(|d| {
                d.get_temp::<u64>(egui::Id::unique("layer-drag")).is_some()
                    || d.get_temp::<FxDrag>(fx_drag_key()).is_some()
                    || d.get_temp::<crate::smart_ui::FilterDrag>(crate::smart_ui::drag_key()).is_some()
            });
            let dragging = held && ctx.input(|i| i.pointer.primary_down());
            let pointer = ctx.input(|i| i.pointer.interact_pos());
            let delta = layer_drag_edge_scroll(pointer, ui.clip_rect(), dragging, ctx.input(|i| i.stable_dt));
            if delta != 0.0 {
                ui.scroll_with_delta(vec2(0.0, delta));
                ctx.request_repaint();
            }
            let filter = crate::layer_filter_ui::Filter::of(&app.ui);
            let fx_collapsed = app.session.active().map(|d| d.fx_collapsed.clone()).unwrap_or_default();
            crate::layer_row_ui::begin(ui.ctx());
            // The drawn rows, top to bottom (#1721): the space between them and below the last
            // one is a drop zone too.
            let mut drawn: Vec<(LayerId, Rect)> = Vec::new();
            for &(depth, l) in &rows {
                // Select › Isolate Layers.
                if !photocraft_engine::select_extra_cmds::isolation_shows(&doc, &isolated, l.id) {
                    continue;
                }
                if !filter.shows(l) {
                    continue;
                }
                let row = RowSel { selected: selection.contains(&l.id), primary: active == Some(l.id), multi: selection.len() > 1 };
                let top = ui.cursor().top();
                drawn.push((l.id, layer_row(app, &ctx, ui, &doc, l, depth, row, &mut actions)));
                if reveal == Some(l.id) {
                    crate::layer_reveal::scroll_to_row(ui, top);
                }
                if !l.effects.items.is_empty() && fx_collapsed.iter().all(|id| *id != l.id) {
                    effect_rows(app, ui, l, depth, &mut actions);
                }
                if !fx_collapsed.contains(&l.id) {
                    crate::smart_ui::filter_rows(app, ui, &doc, l, depth, &mut actions);
                }
            }
            // The bottom of everything drawn: the last layer's effect and Smart Filter rows sit
            // under its own row (#8764).
            let rows_bottom = ui.min_rect().bottom();
            // #1721: the list takes drops everywhere, not only on a row (Photoshop): the space
            // between two rows lands above the lower one, the empty space below the bottom-most
            // row below it. Both copy with ⌥ held, move without.
            for w in drawn.windows(2) {
                let gap = Rect::from_min_max(pos2(w[0].1.left(), w[0].1.bottom()), pos2(w[0].1.right(), w[1].1.top()));
                layer_drop_zone(app, &ctx, ui, gap, (w[1].0, w[1].1), "above", &mut actions);
            }
            if let Some(&(last, last_rect)) = drawn.last() {
                let below = Rect::from_min_max(pos2(last_rect.left(), last_rect.bottom()), pos2(last_rect.right(), ui.clip_rect().bottom()));
                layer_drop_zone(app, &ctx, ui, below, (last, last_rect), "below", &mut actions);
                // A click on that empty space deselects every layer, as in Photoshop (#8100). It
                // starts under the last layer's sub-rows, which take their own clicks.
                let empty = Rect::from_min_max(pos2(below.left(), rows_bottom), below.max);
                if empty.height() > 0.0 && ui.interact(empty, ui.scope_id().with("layers-empty-space"), Sense::click()).clicked() && !selection.is_empty() {
                    actions.push(("select.deselectLayers".into(), json!({})));
                }
            }
            // Tab / ⇧Tab in a rename: rename the layer below / above next, as in Photoshop. The
            // Background isn't renamed in place (double-clicking it converts it), so it ends there.
            if let Some((from, up)) = crate::layer_row_ui::take_next_rename(ui.ctx())
                && let Some(at) = drawn.iter().position(|(id, _)| id.0 == from)
                && let Some(&(next, _)) = if up { at.checked_sub(1).and_then(|i| drawn.get(i)) } else { drawn.get(at + 1) }
                && let Some(l) = doc.layer(next).filter(|l| !crate::doc_props_ui::is_background(&doc, l))
                && let Some(done) = crate::layer_row_ui::start_rename(ui.ctx(), next.0, &l.name)
            {
                actions.push(done);
            }
            // ⌥-click the line between two layers: clip / release the upper one (#967).
            crate::clip_line_ui::show(ui, &doc, &mut actions);
            // A rename whose row is gone (deleted, filtered out, inside a closed group) ends,
            // committed: nothing else could commit or cancel it.
            if let Some(layer) = crate::layer_row_ui::renaming(ui.ctx())
                && !crate::layer_row_ui::recorded(ui.ctx()).iter().any(|r| r.layer == layer)
                && let Some(done) = crate::layer_row_ui::end_rename(ui.ctx(), true)
            {
                actions.push(done);
            }
            // Likewise a selected smart filter whose row is gone.
            crate::smart_ui::forget_hidden(ui.ctx(), doc.id);
        });
    // Leaving the list or hovering an invalid target immediately cancels the temporary
    // image. The release still uses the actual command, not the preview snapshot.
    if !app.reorder_preview_hit {
        app.reorder_preview = None;
    }
    // A layer being dragged can also be dropped on the footer's Delete, New Layer and Group
    // buttons (#736); read the drag before it ends.
    let footer_drag = ctx.data(|d| d.get_temp::<u64>(egui::Id::unique("layer-drag"))).map(|id| {
        let in_selection = selection.iter().any(|s| s.0 == id);
        (id, in_selection)
    });
    // An effect row can be dropped on Delete too (#3102), and a mask's thumbnail (#3113, #3119).
    let fx_footer_drag = ctx.data(|d| d.get_temp::<FxDrag>(fx_drag_key()));
    fx_drag_feedback(&ctx, &doc);
    // And a smart filter's row (#4271).
    let filter_footer_drag = ctx.data(|d| d.get_temp::<crate::smart_ui::FilterDrag>(crate::smart_ui::drag_key()));
    crate::smart_ui::drag_feedback(&ctx, &doc);
    let mask_footer_drag = ctx.data(|d| d.get_temp::<MaskDrag>(mask_drag_key()));
    if let Some((_, kind)) = mask_footer_drag
        && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
    {
        let label = if kind == MaskKind::Vector { tl!("Vector Mask") } else { tl!("Layer Mask") };
        crate::layer_transfer::ghost(&ctx, p, label);
    }
    // End any layer drag after every row has had a chance to accept the drop.
    if ctx.input(|i| i.pointer.any_released()) {
        ctx.data_mut(|d| d.remove::<u64>(egui::Id::unique("layer-drag")));
        ctx.data_mut(|d| d.remove::<FxDrag>(fx_drag_key()));
        ctx.data_mut(|d| d.remove::<MaskDrag>(mask_drag_key()));
        ctx.data_mut(|d| d.remove::<crate::smart_ui::FilterDrag>(crate::smart_ui::drag_key()));
    }
    widgets::panel_footer(ui, |ui| {
        let active_layer = active.and_then(|id| doc.layer(id));
        let (delete, label) = if app.ui.vector_mask_target && active_layer.is_some_and(|l| l.vector_mask.is_some()) {
            ("layer.vectorMask.delete", "Delete Vector Mask")
        } else if app.ui.mask_target && active_layer.is_some_and(|l| l.mask.is_some()) {
            ("layer.layerMask.delete", "Delete Layer Mask")
        } else {
            ("layer.delete", "Delete layer")
        };
        let trash = icons::button(ui, "trash", 26.0, false, tl!(label));
        // A selected smart filter row is what Delete removes, as in Photoshop (#4751).
        let picked_filter = crate::smart_ui::selected(&ctx, &doc, active);
        if trash.clicked()
            && let Some(picked) = picked_filter
        {
            actions.push(crate::smart_ui::trash_action(picked));
            crate::smart_ui::deselect(&ctx, doc.id);
        } else if trash.clicked() {
            // Like Photoshop, ⌥/Alt-click deletes without asking.
            let confirm = delete == "layer.delete" && !ui.input(|i| i.modifiers.alt);
            let params = if confirm { json!({"__trash": true}) } else { json!({}) };
            actions.push((delete.into(), params));
        }
        actions.extend(footer_drop(ui, &trash, footer_drag, "layer.delete"));
        actions.extend(fx_trash_drop(ui, &trash, fx_footer_drag));
        if let Some(drag) = filter_footer_drag
            && dropped_on(ui, &trash)
        {
            actions.push(crate::smart_ui::trash_action(drag));
        }
        // Photoshop asks "Apply mask to layer before removing?" or "Delete the vector mask?"
        // (`mask_trash_prompt`).
        if let Some((layer, kind)) = mask_footer_drag
            && dropped_on(ui, &trash)
        {
            actions.push(crate::mask_trash_prompt::drop_action(layer, kind));
        }
        // A click follows the selected thumbnail; name that action for screen readers even
        // though dropping a whole layer row onto the same button always deletes the layer.
        trash.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!(label)));
        let new_layer = icons::button(ui, "square-plus", 26.0, false, &crate::shortcuts::tip_label(app, "Create a new layer", "layer.new.layer"));
        if new_layer.clicked() {
            // A click makes the layer above the selection; ⌥-click asks first (the New Layer
            // dialog) and ⌘/Ctrl-click adds it below, as in Photoshop.
            let (alt, below) = ui.input(|i| (i.modifiers.alt, i.modifiers.command));
            let params = if alt {
                Value::Null
            } else if below {
                json!({"position": "below"})
            } else {
                json!({})
            };
            actions.push(("layer.new.layer".into(), params));
        }
        actions.extend(footer_drop(ui, &new_layer, footer_drag, "layer.duplicate"));
        let group = icons::button(ui, "folder", 26.0, false, tl!("Create a new group"));
        if group.clicked() {
            // Multiple selected layers share one group. Shift also groups a single selection.
            let shift = ui.input(|i| i.modifiers.shift);
            let cmd = if (shift || selection.len() > 1) && active_layer.is_some() { "layer.groupLayers" } else { "layer.new.group" };
            actions.push((cmd.into(), json!({})));
        }
        actions.extend(footer_drop(ui, &group, footer_drag, "layer.groupLayers"));
        let adj = footer_menu_button(ui, "adjustment-layer", 26.0, tl!("Create new fill or adjustment layer"));
        // Opens on the press like Photoshop's: drag onto an item and release to choose it (#2071).
        crate::press_menu::show(&adj, |ui, menu| {
            ui.set_min_width(190.0);
            for c in photocraft_engine::command_specs().iter().filter(|c| c.id.starts_with("layer.newAdjustmentLayer.")) {
                if menu.item(ui, tl!(c.label).trim_end_matches('…')) {
                    actions.push((c.id.into(), json!({})));
                }
            }
            ui.separator();
            if menu.item(ui, tl!("Solid Color…")) {
                actions.push(("layer.newFillLayer.solidColor".into(), json!({})));
            }
            if menu.item(ui, tl!("Gradient…")) {
                actions.push(("layer.newFillLayer.gradient".into(), json!({})));
            }
        });
        // Like Photoshop the button never replaces a mask (#2075): with a layer mask it adds a
        // vector mask, and with both it is greyed.
        let alt = ui.input(|i| i.modifiers.alt);
        let mask_cmd = crate::layer_menu_ui::mask_button_command(active_layer, doc.selection.is_some(), alt);
        let mask_tip = if active_layer.is_some_and(|l| l.mask.is_some()) {
            tl!("Add vector mask").to_string()
        } else {
            crate::i18n::fmt(tl!("Add a mask  (from the selection; {key} inverts)"), &[("key", &crate::shortcuts::pretty("Alt"))])
        };
        let mask_btn = ui.add_enabled_ui(mask_cmd.is_some(), |ui| icons::button(ui, "layer-mask", 26.0, false, &mask_tip)).inner;
        mask_btn.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &mask_tip));
        if mask_btn.clicked()
            && let Some(cmd) = mask_cmd
        {
            // A path selected in the Paths panel becomes the vector mask, as in Photoshop (#4728);
            // Alt still inverts it.
            match crate::vector_ui::selected_document_path(app).filter(|_| cmd.starts_with("layer.vectorMask.")) {
                Some(name) => actions.push(("layer.vectorMask.currentPath".into(), json!({"name": name, "hide": alt}))),
                None => actions.push((cmd.into(), json!({}))),
            }
        }
        let fx = fx_button(ui, 26.0, tl!("Add a layer style"));
        crate::press_menu::show(&fx, |ui, menu| {
            ui.set_min_width(180.0);
            if menu.item(ui, tl!("Blending Options…")) {
                crate::layer_style::open(app, None);
            }
            ui.separator();
            for &(kind, label) in crate::layer_style::KINDS {
                if menu.item(ui, format!("{}…", tl!(label))) {
                    crate::layer_style::open(app, Some(kind));
                }
            }
        });
        // Photoshop's footer starts with Link Layers (enabled with two or more layers selected).
        let can_link = app.session.is_enabled("layer.linkLayers");
        if ui.add_enabled_ui(can_link, |ui| icons::button(ui, "link-2", 26.0, false, tl!("Link layers"))).inner.clicked() {
            actions.push(("layer.linkLayers".into(), json!({})));
        }
    });
    for (id, p) in actions {
        // A row click commits even when it selects the layer already being edited (#5584).
        // Finish before selection changes so the existing type session keeps its own target.
        if id == "layer.select" && app.ui.text_edit.is_some() {
            crate::type_tool::commit(app);
        }
        if id == "ui.maskTarget" {
            app.end_filter_mask_view();
            app.ui.mask_target = p.as_bool().unwrap_or(false);
            app.ui.vector_mask_target = false;
            app.ui.filter_mask_target = None;
            app.sync_mask_targets();
            continue;
        }
        if id == "ui.vectorMaskTarget" {
            app.end_filter_mask_view();
            app.ui.vector_mask_target = p.as_bool().unwrap_or(false);
            app.ui.mask_target = false;
            app.ui.filter_mask_target = None;
            app.sync_mask_targets();
            continue;
        }
        if id == "ui.filterMaskTarget" {
            app.select_filter_mask_target(p.as_u64());
            continue;
        }
        // Double-clicking a mask thumbnail opens its settings: show Properties.
        if id == "ui.properties" {
            app.ui.panels.properties = true;
            app.ui.dock_tabs.properties = 0;
            app.ui.dock.collapsed.retain(|g| *g != crate::dock::Group::Properties);
            continue;
        }
        if p.is_null() || id == "layer.newFillLayer.solidColor" {
            // Context-menu items and the Layers footer's Solid Color… action behave like
            // their menu-bar twins. In particular, a new Solid Color fill opens its picker.
            let ctx = ui.ctx().clone();
            let params = if p.is_null() { json!({}) } else { p };
            if let Err(e) = crate::menus::invoke(app, &ctx, &id, params) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
            continue;
        }
        let _ = app.run(&id, p);
    }
}

pub(super) fn footer_label(command: &str) -> &'static str {
    match command {
        "layer.delete" => tl!("Delete layer"),
        "layer.duplicate" => tl!("Create a new layer"),
        _ => tl!("Create a new group"),
    }
}

/// The small down-arrow makes it clear that the footer icon expands into a menu.
fn footer_menu_arrow(ui: &egui::Ui, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    let arrow = Rect::from_min_size(rect.right_bottom() - vec2(11.0, 10.0), vec2(9.0, 9.0));
    icons::paint(ui, arrow, "chevron-down", 8.0, t.text_dim);
}

fn footer_menu_button(ui: &mut egui::Ui, icon: &str, size: f32, tip: &str) -> egui::Response {
    let response = icons::button(ui, icon, size, false, tip);
    // Named for screen readers (and tests) after its tooltip.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    footer_menu_arrow(ui, response.rect);
    response
}

/// Photoshop's italic "fx" footer button (no icon-font equivalent).
fn fx_button(ui: &mut egui::Ui, size: f32, tip: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(r, t.radius_sm, t.hover);
    }
    let mut job = egui::text::LayoutJob::default();
    job.append("fx", 0.0, egui::TextFormat { font_id: egui::FontId::proportional(15.0), color: t.icon, italics: true, ..Default::default() });
    let g = ui.painter().layout_job(job);
    ui.painter().galley(r.center() - g.size() / 2.0, g, t.icon);
    footer_menu_arrow(ui, r);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    resp.on_hover_text(tip)
}

pub(super) fn channels(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::channels_panel::show(app, ui);
}
