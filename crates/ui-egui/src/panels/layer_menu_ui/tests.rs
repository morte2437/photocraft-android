use super::*;

#[test]
fn mask_toggle_label_follows_history_and_selected_layer() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let mut app = crate::PhotocraftApp::new(s, Default::default());
    let toggle = "layer.layerMask.enabled";
    let check = |app: &crate::PhotocraftApp, label: Option<&str>| {
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        let entry = entries(l, false, st.doc.selection.is_some()).into_iter().flatten().find(|e| e.1 == toggle);
        assert_eq!(entry.map(|e| e.0), label);
        // A selection has no single mask to toggle: its menu never offers the item.
        assert!(entries(l, true, st.doc.selection.is_some()).into_iter().flatten().all(|e| e.1 != toggle));
        let item = crate::menus::menu_items(app).into_iter().find(|i| i.id == toggle).unwrap();
        assert_eq!(item.enabled, label.is_some());
        assert_eq!(item.label, label.unwrap_or("Enable Layer Mask"));
        for lang in [crate::i18n::Lang::EN, crate::i18n::Lang::from_pref("fr")] {
            let layout = crate::native_menu::photocraft_layout(&crate::menus::menu_items(app), lang, lang.code());
            let native = layout.bar.find(toggle).unwrap();
            assert_eq!(native.label, crate::i18n::tr(lang, &item.label));
            assert_eq!(native.enabled, item.enabled);
        }
    };
    check(&app, None);
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    let masked = app.session.active().unwrap().active_layer.unwrap();
    check(&app, Some("Disable Layer Mask"));
    app.run(toggle, json!({})).unwrap();
    check(&app, Some("Enable Layer Mask"));
    app.run("edit.undo", json!({})).unwrap();
    check(&app, Some("Disable Layer Mask"));
    app.run("edit.redo", json!({})).unwrap();
    check(&app, Some("Enable Layer Mask"));
    app.run("layer.new.layer", json!({})).unwrap();
    check(&app, None);
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    check(&app, Some("Disable Layer Mask"));
    app.run("layer.select", json!({"layer": masked.0, "mode": "add"})).unwrap();
    check(&app, Some("Enable Layer Mask"));
    app.run("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
    check(&app, Some("Enable Layer Mask"));
    app.session.active_mut().unwrap().active_layer = None;
    assert!(!crate::menus::is_enabled(&app, toggle));
    let empty = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    assert!(!crate::menus::is_enabled(&empty, toggle));
}

#[test]
fn mask_toggle_context_menu_follows_the_clicked_layer_and_existing_translations() {
    use egui_kittest::{Harness, kittest::Queryable};

    for language in ["en", "fr"] {
        let lang = crate::i18n::Lang::from_pref(language);
        let _language = crate::i18n::language_scope(lang);
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let clicked = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let other = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("layer.layerMask.enabled", json!({})).unwrap();
        let app = crate::PhotocraftApp::new(s, Default::default());
        // `other` stays the active layer with its mask disabled; the menu is opened over `clicked`,
        // whose mask is enabled, and selects it first (a single-layer menu).
        let mut h = Harness::builder().with_size(egui::vec2(400.0, 900.0)).build_ui_state(
            move |ui, app| {
                let st = app.session.active().unwrap();
                let l = st.doc.layer(clicked).unwrap().clone();
                let mut actions = Vec::new();
                show(app, ui, &l, false, &mut actions);
                for (id, params) in actions {
                    crate::menus::invoke(app, ui.ctx(), &id, params).unwrap();
                }
            },
            app,
        );
        h.run_steps(3);
        let enabled_label = crate::i18n::tr(lang, "Enable Layer Mask");
        let disabled_label = crate::i18n::tr(lang, "Disable Layer Mask");
        // The label follows the mask of the clicked layer, not of the active one.
        assert!(h.query_by_label(enabled_label).is_none());
        h.get_by_label(disabled_label).click();
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().active_layer, Some(clicked));
        assert!(!h.state().session.active().unwrap().doc.layer(clicked).unwrap().mask.as_ref().unwrap().enabled);
        assert!(!h.state().session.active().unwrap().doc.layer(other).unwrap().mask.as_ref().unwrap().enabled);
        h.get_by_label(enabled_label);
    }
}

/// With several layers selected there is no single mask to toggle, apply or delete: the menu
/// offers Add Layer Mask only, whatever the clicked layer has, and does not hit the active layer.
#[test]
fn multi_selection_menu_has_no_per_layer_mask_items() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let st = s.active().unwrap();
    let masked = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
    let ids = entries(&masked, true, false).into_iter().flatten().map(|e| e.1).collect::<Vec<_>>();
    assert!(ids.contains(&"layer.layerMask.revealAll"));
    for single in ["layer.layerMask.enabled", "layer.layerMask.apply", "layer.layerMask.delete"] {
        assert!(!ids.contains(&single), "{single} acts on one layer, not on a selection");
    }
}

#[test]
fn smart_object_context_menu_edits_the_clicked_layer() {
    use crate::PhotocraftApp;
    use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
    use egui_kittest::{Harness, kittest::Queryable};

    fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, button: PointerButton) {
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
    }

    for ppp in [1.0, 2.0] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 640, "height": 480})).unwrap();
        s.execute("layer.new.layer", json!({"name": "Smart source"})).unwrap();
        s.execute("paint.stroke", json!({"points": [[100, 100]], "size": 20})).unwrap();
        let smart = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("layer.new.layer", json!({"name": "Other active layer"})).unwrap();
        let mut h =
            Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(ppp).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                PhotocraftApp::new(s, crate::Services::default())
            });
        let ctx = h.ctx.clone();
        let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
        crate::control::handle(h.state_mut(), &ctx, &req);
        h.run_steps(8);
        let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == smart).expect("layer row drawn").row;
        // Top-level Pro row: 6 pt padding, 28 pt eye column, 24 pt layer thumbnail.
        let at = pos2(row.left() + 6.0 + 28.0 + 12.0, row.center().y);
        click(&mut h, at, PointerButton::Secondary);
        let at = h.get_by_label("Edit Contents").rect().center();
        click(&mut h, at, PointerButton::Primary);
        assert_eq!(h.state().session.active_index(), Some(1), "@{ppp}x: contents tab opens");
        assert!(h.state().session.is_enabled("layer.smartObjects.saveContents"));
        assert!(h.state().session.active().unwrap().doc.walk().iter().any(|(_, _, l)| l.name == "Smart source"));
        h.state_mut().session.set_active(0);
        assert_eq!(h.state().session.active().unwrap().active_layer, Some(photocraft_doc::LayerId(smart)));
    }
}

/// #4575: right-clicking a layer-mask thumbnail opens Photoshop's mask menu, not the layer
/// menu, and its selection items load the clicked layer's mask (not the active layer's).
#[test]
fn mask_thumbnail_context_menu_acts_on_the_clicked_mask() {
    use crate::PhotocraftApp;
    use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
    use egui_kittest::{Harness, kittest::Queryable};

    fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, button: PointerButton) {
        h.hover_at(at);
        h.step();
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
    }

    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 100, "height": 60})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Masked"})).unwrap();
    s.execute("select.rect", json!({"x": 60, "y": 0, "width": 40, "height": 60})).unwrap();
    s.execute("layer.layerMask.revealSelection", json!({})).unwrap();
    let masked = s.active().unwrap().active_layer.unwrap();
    // Another layer is active, and the selection doesn't overlap the mask.
    s.execute("layer.new.layer", json!({"name": "Other active layer"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 60})).unwrap();
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    let (thumbs, _) = crate::mask_thumbs_ui::recorded(&h.ctx, masked.0).expect("mask thumbnail drawn");
    let at = thumbs.iter().find(|(k, _)| *k == crate::mask_thumbs_ui::MaskKind::Pixel).unwrap().1.center();
    click(&mut h, at, PointerButton::Secondary);
    assert!(h.query_by_label("Blending Options…").is_none(), "the mask menu, not the layer menu");
    for item in
        ["Disable Layer Mask", "Delete Layer Mask", "Apply Layer Mask", "Subtract Mask from Selection", "Intersect Mask with Selection", "Select and Mask…"]
    {
        h.get_by_label(item);
    }
    let at = h.get_by_label("Add Mask to Selection").rect().center();
    click(&mut h, at, PointerButton::Primary);
    let st = h.state().session.active().unwrap();
    assert_eq!(st.active_layer, Some(masked));
    let sel = st.doc.selection.as_ref().expect("a selection");
    assert!(sel.sample_channel(10, 30, 0) > 0.99, "the selection is kept");
    assert!(sel.sample_channel(80, 30, 0) > 0.99, "the clicked layer's mask is added");
    assert!(sel.sample_channel(40, 30, 0) < 0.01, "nothing else is selected");
    // Mask Options…: Layer Mask Display Options (the rubylith's colour and opacity).
    let (thumbs, _) = crate::mask_thumbs_ui::recorded(&h.ctx, masked.0).expect("mask thumbnail drawn");
    let at = thumbs.iter().find(|(k, _)| *k == crate::mask_thumbs_ui::MaskKind::Pixel).unwrap().1.center();
    click(&mut h, at, PointerButton::Secondary);
    let at = h.get_by_label("Mask Options…").rect().center();
    click(&mut h, at, PointerButton::Primary);
    let d = h.state().ui.dialogs.last().expect("Mask Options… opens a dialog");
    assert!(crate::channel_options_ui::owns(&d.fields) && d.fields["channel"] == json!("mask"), "{:?}", d.fields);
    assert_eq!(d.fields["__label"], json!("Layer Mask Display Options…"));
}

#[test]
fn a_long_layer_menu_stays_inside_a_short_window_and_scrolls() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap();
    let app = crate::PhotocraftApp::new(s, crate::Services::default());
    let layer = {
        let st = app.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
    };
    let ctx = egui::Context::default();
    crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 300.0));
    let id = egui::Id::unique("layer-menu-probe");
    let mut content = 0.0;
    for _ in 0..4 {
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            // Opened low in the window, like a right-click on a bottom Layers row.
            egui::Area::new(id).order(egui::Order::Foreground).default_pos(egui::pos2(300.0, 250.0)).show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    let start = ui.next_widget_position().y;
                    show(&app, ui, &layer, false, &mut Vec::new());
                    content = ui.min_rect().bottom() - start;
                });
            });
        });
        out.textures_delta.clear();
    }
    let rect = ctx.memory(|m| m.area_rect(id)).expect("the menu was shown");
    assert!(screen.contains_rect(rect), "the menu moves up and stays inside the window: {rect:?}");
    assert!(rect.height() < screen.height(), "{rect:?}");
    let rows = entries(&layer, false, false).iter().flatten().count();
    assert!(rows >= 15, "Photoshop's layer menu is long: {rows} rows");
    assert!(content < rect.height() + 1.0, "the rows scroll inside the menu instead of running off the window");
}

#[test]
fn edit_contents_is_available_only_for_smart_objects() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Source"})).unwrap();
    let layer = |s: &photocraft_engine::Session| {
        let st = s.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
    };
    assert!(!entries(&layer(&s), false, false).iter().flatten().any(|e| e.1 == "layer.smartObjects.editContents"));
    s.execute("paint.stroke", json!({"points": [[12, 12]], "size": 8})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let ids: Vec<_> = entries(&layer(&s), false, false).into_iter().flatten().map(|e| e.1).collect();
    let edit = ids.iter().position(|id| *id == "layer.smartObjects.editContents").expect("Edit Contents entry");
    assert_eq!(
        ids[edit + 1..edit + 5],
        ["layer.smartObjects.replaceContents", "layer.smartObjects.relinkToFile", "layer.smartObjects.convertToLayers", "layer.rasterize.smartObject"]
    );
}

/// Right-clicking inside a multi-layer selection offers one Rasterize item for the whole
/// selection, whatever kind the clicked layer is, and no single-layer Smart Object items.
#[test]
fn multi_selection_menu_rasterizes_the_selection() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 32, "height": 24})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Source"})).unwrap();
    s.execute("paint.stroke", json!({"points": [[12, 12]], "size": 8})).unwrap();
    s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let st = s.active().unwrap();
    let smart = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
    let ids = |multi| entries(&smart, multi, false).into_iter().flatten().map(|e| e.1).collect::<Vec<_>>();
    assert!(ids(false).contains(&"layer.smartObjects.editContents"), "a single smart object keeps its items");
    let multi = ids(true);
    assert!(multi.contains(&"layer.rasterize.layer"));
    for single in [
        "layer.rasterize.smartObject",
        "layer.rasterize.shape",
        "layer.smartObjects.editContents",
        "layer.smartObjects.replaceContents",
        "layer.smartObjects.relinkToFile",
        "layer.smartObjects.convertToLayers",
    ] {
        assert!(!multi.contains(&single), "{single} acts on one layer, not on a selection");
    }
    let label = entries(&smart, true, false).into_iter().flatten().find(|e| e.1 == "layer.rasterize.layer").unwrap().0;
    assert_eq!(label, "Rasterize Layers");
}

fn click(h: &mut egui_kittest::Harness<'_, crate::PhotocraftApp>, at: egui::Pos2, button: egui::PointerButton) {
    h.hover_at(at);
    h.step();
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
}

/// A type layer, an ellipse and a rectangle; the type layer and the ellipse are selected (the
/// ellipse active), the rectangle stays out. The Layers menu is opened by right-clicking the
/// type row, which is selected but not the active layer. Returns `[type, ellipse, rectangle]`.
fn open_multi_selection_menu() -> (egui_kittest::Harness<'static, crate::PhotocraftApp>, [u64; 3]) {
    use crate::PhotocraftApp;
    use egui::{PointerButton, pos2, vec2};

    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 640, "height": 480})).unwrap();
    let text = s.execute("type.create", json!({"text": "Hi", "size": 24, "x": 20, "y": 60})).unwrap()["layer"].as_u64().unwrap();
    let shape = s.execute("shape.create", json!({"kind": "ellipse", "rect": [100, 100, 200, 160]})).unwrap()["layer"].as_u64().unwrap();
    let other = s.execute("shape.create", json!({"kind": "rect", "rect": [300, 100, 380, 160]})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer": text})).unwrap();
    s.execute("layer.select", json!({"layer": shape, "mode": "add"})).unwrap();
    let mut h =
        egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).with_pixels_per_point(1.0).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(
            move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                PhotocraftApp::new(s, crate::Services::default())
            },
        );
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == text).expect("layer row drawn").row;
    click(&mut h, pos2(row.left() + 6.0 + 28.0 + 12.0, row.center().y), PointerButton::Secondary);
    (h, [text, shape, other])
}

/// The reported bug: right-click a layer inside a multi-layer selection, choose Rasterize, and
/// only the clicked (or active) layer was converted.
#[test]
fn rasterize_from_the_menu_converts_every_selected_layer() {
    use egui_kittest::kittest::Queryable;
    let (mut h, [text, shape, other]) = open_multi_selection_menu();
    let at = h.get_by_label("Rasterize Layers").rect().center();
    click(&mut h, at, egui::PointerButton::Primary);
    let doc = &h.state().session.active().unwrap().doc;
    let raster = |id: u64| matches!(doc.layer(photocraft_doc::LayerId(id)).unwrap().content, photocraft_doc::LayerContent::Raster(_));
    assert!(raster(text) && raster(shape), "both selected layers are rasterized");
    assert!(!raster(other), "an unselected layer is left alone");
    assert_eq!(h.state().session.active().unwrap().selected_layers().len(), 2, "the selection survives");
}

/// Group from Layers is offered for a multi-layer selection too (it used to be dropped from
/// the menu then). As in Photoshop it asks in the New Group from Layers dialog first; OK puts
/// every selected layer in one new group, leaving the rest alone.
#[test]
fn group_from_layers_groups_every_selected_layer() {
    use egui_kittest::kittest::Queryable;
    use photocraft_doc::LayerId;
    let (mut h, [text, shape, other]) = open_multi_selection_menu();
    let at = h.get_by_label("Group from Layers…").rect().center();
    click(&mut h, at, egui::PointerButton::Primary);
    let dialog = h.state().ui.dialogs.last().map(|d| (d.id, d.fields.clone())).expect("the New Group dialog opened");
    assert!(crate::new_group_ui::owns(&dialog.1));
    assert_eq!(dialog.1["__command"], "layer.new.groupFromLayers");
    crate::dialogs::confirm(h.state_mut(), dialog.0).unwrap();
    h.run_steps(2);
    let doc = &h.state().session.active().unwrap().doc;
    let groups: Vec<_> = doc.walk().into_iter().map(|(_, _, l)| l).filter(|l| l.is_group()).collect();
    assert_eq!(groups.len(), 1);
    let children: Vec<LayerId> = groups[0].children().unwrap().iter().map(|l| l.id).collect();
    assert_eq!(children, vec![LayerId(text), LayerId(shape)], "both selected layers, in order");
    assert!(doc.layers.iter().any(|l| l.id == LayerId(other)), "an unselected layer stays at the top level");
}

/// Items with no single target are greyed for a multi-layer selection (they used to run on
/// whichever layer happened to be active, not the one right-clicked); the ones that act on the
/// whole selection, including adding a mask, stay available.
#[test]
fn multi_selection_menu_greys_what_acts_on_one_layer() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let (h, _) = open_multi_selection_menu();
    for label in ["Blending Options…", "Copy Layer Style"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} works on one layer only");
    }
    for label in [
        "Quick Export As PNG",
        "Export As…",
        "Add Layer Mask",
        "Duplicate Layers…",
        "Delete Layers",
        "Group from Layers…",
        "Convert to Smart Object",
        "Rasterize Layers",
        "Link Layers",
        "Merge Layers",
    ] {
        assert!(!h.get_by_label(label).accesskit_node().is_disabled(), "{label} acts on the selection");
    }
    // Greyed here only for lack of something to act on: no style on the clipboard, no style to clear.
    for label in ["Paste Layer Style", "Clear Layer Style"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} has nothing to do here");
    }
    // Whatever the clicked layer is, the menu has the same shape: no per-layer mask or Smart Object items.
    for label in ["Disable Layer Mask", "Apply Layer Mask", "Delete Layer Mask", "Edit Contents", "Convert to Layers"] {
        assert!(h.query_by_label(label).is_none(), "{label} is not offered for several layers");
    }
}

/// Every entry of the multi-layer menu is either known to act on the selection or greyed by
/// [`single_layer_only`]; a new single-layer command added to the menu must be classified.
#[test]
fn single_layer_only_covers_the_per_layer_commands() {
    for id in
        ["layer.layerStyle.blendingOptions", "layer.layerStyle.copyLayerStyle", "layer.layerMask.enabled", "layer.layerMask.apply", "layer.layerMask.delete"]
    {
        assert!(single_layer_only(id), "{id}");
    }
    for id in [
        "layer.quickExportAsPng",
        "layer.exportAs",
        "layer.duplicate",
        "layer.delete",
        "layer.new.groupFromLayers",
        "layer.rasterize.layer",
        "layer.smartObjects.convertToSmartObject",
        "layer.createClippingMask",
        "layer.linkLayers",
        "layer.mergeLayers",
        "layer.layerMask.revealAll",
        "layer.layerMask.hideSelection",
        "layer.layerStyle.pasteLayerStyle",
        "layer.layerStyle.clear",
    ] {
        assert!(!single_layer_only(id), "{id}");
    }
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let st = s.active().unwrap();
    let masked = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
    let ids: Vec<_> = entries(&masked, true, false).into_iter().flatten().map(|e| e.1).collect();
    assert!(ids.contains(&"layer.layerMask.revealAll") && !ids.contains(&"layer.layerMask.apply"), "one stable mask item for several layers");
}

#[test]
fn entries_follow_layer_state() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let st = s.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
    let ids: Vec<&str> = entries(&l, false, false).into_iter().flatten().map(|e| e.1).collect();
    assert_eq!(ids.first(), Some(&"layer.layerStyle.blendingOptions"));
    assert!(ids.contains(&"layer.mergeDown") && !ids.contains(&"layer.mergeLayers"));
    assert!(ids.contains(&"layer.layerMask.revealAll"));
    let ids: Vec<&str> = entries(&l, false, true).into_iter().flatten().map(|e| e.1).collect();
    assert!(ids.contains(&"layer.layerMask.revealSelection") && !ids.contains(&"layer.layerMask.revealAll"));
    let ids: Vec<&str> = entries(&l, true, false).into_iter().flatten().map(|e| e.1).collect();
    assert!(ids.contains(&"layer.mergeLayers"));
}

/// #3832: a group's right-click menu and the Layer menu offer Photoshop's Merge Group, which
/// merges the group itself (not into the layer below), and it is enabled.
#[test]
fn a_group_offers_merge_group() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("layer.groupLayers", json!({"name": "G"})).unwrap();
    let st = s.active().unwrap();
    let g = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
    let e: Vec<(&str, &str)> = entries(&g, false, false).into_iter().flatten().collect();
    assert!(e.contains(&("Merge Group", "layer.mergeGroup")), "{e:?}");
    assert!(!e.iter().any(|x| x.0 == "Merge Down"));
    let mut app = crate::PhotocraftApp::new(s, Default::default());
    app.sync_views();
    let items = crate::menus::menu_items(&app);
    let item = items.iter().find(|i| i.id == "layer.mergeLayers").unwrap();
    assert_eq!(item.label, "Merge Group");
    assert!(item.enabled, "Merge Group runs on a group");
    // A plain layer keeps Merge Layers.
    app.run("layer.new.layer", json!({})).unwrap();
    let items = crate::menus::menu_items(&app);
    assert_eq!(items.iter().find(|i| i.id == "layer.mergeLayers").unwrap().label, "Merge Layers");
}

#[test]
fn add_mask_uses_the_selection() {
    assert_eq!(add_mask_command(false, false), "layer.layerMask.revealAll");
    assert_eq!(add_mask_command(false, true), "layer.layerMask.hideAll");
    assert_eq!(add_mask_command(true, true), "layer.layerMask.hideSelection");
    // With a selection the new mask is the selection: inside revealed, outside hidden.
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 4, "height": 10})).unwrap();
    s.execute(add_mask_command(true, false), json!({})).unwrap();
    let st = s.active().unwrap();
    let mask = &st.doc.layer(st.active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface;
    assert!(mask.pixel(1, 5)[0] > 0.99 && mask.pixel(8, 5)[0] < 0.01);
}

/// #2075: clicking the mask button again (with or without ⌥) used to replace the layer's
/// mask, discarding what was painted into it. Like Photoshop, it now adds a vector mask, and
/// does nothing once the layer has both.
#[test]
fn mask_button_never_replaces_a_mask() {
    for alt in [false, true] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let layer = |s: &photocraft_engine::Session| {
            let st = s.active().unwrap();
            st.doc.layer(st.active_layer.unwrap()).unwrap().clone()
        };
        let first = mask_button_command(Some(&layer(&s)), false, alt).unwrap();
        assert_eq!(first, if alt { "layer.layerMask.hideAll" } else { "layer.layerMask.revealAll" });
        s.execute(first, json!({})).unwrap();
        // Paint a stroke into the mask.
        s.edit("paint mask", |doc, active| {
            let id = (*active).ok_or(photocraft_engine::EngineError::NoDocument)?;
            let m = doc.layer_mut(id).and_then(|l| l.mask.as_mut()).ok_or(photocraft_engine::EngineError::NoDocument)?;
            m.surface.fill_rect(photocraft_geom::Rect::new(2, 2, 5, 5), &[0.5]);
            Ok(())
        })
        .unwrap();
        let painted = layer(&s).mask.unwrap().surface.pixel(3, 3)[0];
        assert!((painted - 0.5).abs() < 0.01);
        // Second click: a vector mask (⌥ hides all), the layer mask is untouched.
        let second = mask_button_command(Some(&layer(&s)), false, alt).unwrap();
        assert_eq!(second, if alt { "layer.vectorMask.hideAll" } else { "layer.vectorMask.revealAll" });
        s.execute(second, json!({})).unwrap();
        let l = layer(&s);
        assert!(l.vector_mask.is_some(), "alt={alt}");
        let mask = l.mask.as_ref().unwrap();
        assert!((mask.surface.pixel(3, 3)[0] - painted).abs() < 1e-6, "alt={alt}: the painted mask was replaced");
        // Third click: nothing left to add.
        assert_eq!(mask_button_command(Some(&l), false, alt), None);
        assert_eq!(mask_button_command(Some(&l), true, alt), None);
    }
    // No layer: nothing to do.
    assert_eq!(mask_button_command(None, false, false), None);
    // A shape layer's path is already its vector mask: after the layer mask, nothing more.
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    s.execute("shape.create", json!({"kind": "rect", "rect": [0, 0, 5, 5]})).unwrap();
    s.execute("layer.layerMask.revealAll", json!({})).unwrap();
    let st = s.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(mask_button_command(Some(l), false, false), None);
}

#[test]
fn every_entry_is_a_known_command() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 10, "height": 10})).unwrap();
    let l = s.active().unwrap().doc.layers[0].clone();
    for (_, id) in entries(&l, false, true).into_iter().chain(entries(&l, false, false)).chain(entries(&l, true, false)).chain(effects_entries()).flatten() {
        assert!(crate::menu_catalog::CATALOG.iter().any(|m| m.3 == id) || photocraft_engine::commands::find(id).is_some(), "{id}");
    }
}
