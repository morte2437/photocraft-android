use super::*;

#[test]
fn modifier_operations() {
    let m = |shift, alt| egui::Modifiers { command: true, shift, alt, ..Default::default() };
    assert_eq!(load_operation(m(false, false)), "new");
    assert_eq!(load_operation(m(true, false)), "add");
    assert_eq!(load_operation(m(false, true)), "subtract");
    assert_eq!(load_operation(m(true, true)), "intersect");
}

#[test]
fn rename_field_interrupts_the_ime_only_when_it_takes_focus() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("channel.new", json!({})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.run_ui(Default::default(), |_| {}).textures_delta.clear();
    let doc = app.session.active().unwrap().doc.id.0;
    let name = app.session.active().unwrap().doc.channels[0].name.clone();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("chan-rename", doc, format!("{:?}", Row::Alpha(0).reference()))), name));
    // The field takes focus on the first pass and owns the IME from the second on (#585).
    let interrupts: Vec<Option<bool>> = (0..4)
        .map(|_| {
            let mut out = ctx.run_ui(Default::default(), |ui| show(&mut app, ui));
            out.textures_delta.clear();
            out.platform_output.ime.map(|ime| ime.should_interrupt_composition)
        })
        .collect();
    assert_eq!(interrupts, [None, Some(false), Some(false), Some(false)]);
}

#[test]
fn channel_thumbs_honours_show_channels_in_color() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    let ctx = egui::Context::default();
    let thumbs_gray = app.channel_thumbs(&ctx);
    assert!(!thumbs_gray.is_empty());

    app.session.execute("prefs.set", json!({"values": {"interface.showChannelsInColor": true}})).unwrap();
    let thumbs_color = app.channel_thumbs(&ctx);
    assert_eq!(thumbs_gray.len(), thumbs_color.len());
}

/// A selection-only step keeps the thumbnails; a pixel edit rebuilds them into the same
/// textures rather than new ones (#4518).
#[test]
fn channel_thumbs_skip_selection_steps_and_reuse_textures() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    let ctx = egui::Context::default();
    // Texture uploads one pass makes (new textures and updates both count).
    let pass = |app: &mut PhotocraftApp| {
        ctx.begin_pass(Default::default());
        let ids = app.channel_thumbs(&ctx);
        let mut delta = ctx.end_pass().textures_delta;
        let n = delta.set.len();
        delta.clear();
        (ids, n)
    };
    let (first, uploads) = pass(&mut app);
    assert!(uploads > 0);
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 20, "height": 20})).unwrap();
    assert_eq!(pass(&mut app), (first.clone(), 0), "a marquee changes no pixels");
    app.run("select.deselect", json!({})).unwrap();
    assert_eq!(pass(&mut app), (first.clone(), 0), "neither does Deselect");
    app.run("edit.fill", json!({"color": "#336699"})).unwrap();
    let (after, uploads) = pass(&mut app);
    assert!(uploads > 0, "a fill rebuilds them");
    assert_eq!(after, first, "into the same textures");
}

/// The Channels tab (the Layers card's second tab) of a 64×48 white document with a layer
/// mask on "Masked" and one alpha channel.
fn channels_harness() -> (egui_kittest::Harness<'static, PhotocraftApp>, u64) {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    let masked = s.execute("layer.new.layer", json!({"name": "Masked"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.layerMask.revealAll", json!({"layer": masked})).unwrap();
    s.execute("channel.new", json!({})).unwrap();
    // 60 fps steps, so two clicks a frame apart count as a double-click.
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 1000.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (req, _rx) = crate::control::ControlRequest::new(
        "ui.set",
        json!({"dockWidth": 290.0, "dockTabs": {"layers": 1}, "dock": {"collapsed": ["color", "properties", "history", "navigator"]}}),
    );
    crate::control::handle(h.state_mut(), &ctx, &req);
    h.run_steps(8);
    (h, masked)
}

/// The centre of the row or button labelled `label` (not its tooltip, which a hover leaves open).
fn at(h: &egui_kittest::Harness<'_, PhotocraftApp>, label: &str) -> egui::Pos2 {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT, Queryable};
    h.query_all_by_label(label)
        .find(|n| matches!(n.accesskit_node().role(), Role::Button | Role::ListBoxOption | Role::Label) && n.rect().height() > 20.0)
        .unwrap_or_else(|| panic!("no row or button {label}"))
        .rect()
        .center()
}

/// A real press, drag and release.
fn drag(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
    use egui::{Modifiers, PointerButton};
    h.hover_at(from);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=6 {
        h.hover_at(from + (to - from) * (i as f32 / 6.0));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

/// Drags the row or button labelled `from` onto the one labelled `to`.
fn drag_to(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, from: &str, to: &str) {
    let (from, to) = (at(h, from), at(h, to));
    drag(h, from, to);
}

fn alphas(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> Vec<String> {
    h.state().session.active().unwrap().doc.channels.iter().map(|c| c.name.clone()).collect()
}

/// #9163: alpha channels reorder by dragging, as in Photoshop: a drop in a row's upper half
/// lands above it, over the colour channels it lands first among the alphas, colour channels
/// don't move, and each reorder is one history step.
#[test]
fn alpha_channels_reorder_by_dragging() {
    let (mut h, _) = channels_harness();
    h.state_mut().run("channel.new", json!({"name": "Alpha 2"})).unwrap();
    h.state_mut().run("channel.new", json!({"name": "Alpha 3"})).unwrap();
    h.run_steps(3);
    assert_eq!(alphas(&h), ["Alpha 1", "Alpha 2", "Alpha 3"]);
    let steps = h.state().session.active().unwrap().history.past_len();

    let (from, onto) = (at(&h, "Alpha 3"), at(&h, "Alpha 1"));
    drag(&mut h, from, onto - vec2(0.0, 8.0));
    assert_eq!(alphas(&h), ["Alpha 3", "Alpha 1", "Alpha 2"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one history step");

    // The lower half of a row: below it.
    let (from, onto) = (at(&h, "Alpha 3"), at(&h, "Alpha 1"));
    drag(&mut h, from, onto + vec2(0.0, 8.0));
    assert_eq!(alphas(&h), ["Alpha 1", "Alpha 3", "Alpha 2"]);

    // Dropped on a colour channel: first among the alphas.
    drag_to(&mut h, "Alpha 2", "Red");
    assert_eq!(alphas(&h), ["Alpha 2", "Alpha 1", "Alpha 3"]);

    // A colour channel dragged onto the alphas stays where it is.
    let before = h.state().session.active().unwrap().history.past_len();
    drag_to(&mut h, "Green", "Alpha 3");
    assert_eq!(alphas(&h), ["Alpha 2", "Alpha 1", "Alpha 3"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), before);
}

/// #4689: as in Photoshop, a channel dropped on New is duplicated, on Delete deleted, and on
/// Load Channel as Selection loaded.
#[test]
fn rows_dropped_on_the_footer_duplicate_delete_and_load() {
    let (mut h, _) = channels_harness();
    assert_eq!(alphas(&h), ["Alpha 1"]);
    let steps = h.state().session.active().unwrap().history.past_len();

    drag_to(&mut h, "Alpha 1", "Create new channel");
    assert_eq!(alphas(&h), ["Alpha 1", "Alpha 1 copy"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1, "one history step");
    drag_to(&mut h, "Red", "Create new channel");
    assert_eq!(alphas(&h), ["Alpha 1", "Alpha 1 copy", "Red copy"]);
    drag_to(&mut h, "Masked Mask", "Create new channel");
    assert_eq!(alphas(&h), ["Alpha 1", "Alpha 1 copy", "Red copy", "Masked Mask copy"]);

    drag_to(&mut h, "Alpha 1 copy", "Delete current channel");
    assert_eq!(alphas(&h), ["Alpha 1", "Red copy", "Masked Mask copy"]);
    // Colour channels stay: Photoshop would turn the image into Multichannel.
    drag_to(&mut h, "Green", "Delete current channel");
    assert_eq!(alphas(&h), ["Alpha 1", "Red copy", "Masked Mask copy"]);
    assert_eq!(h.state().session.active().unwrap().doc.mode, photocraft_doc::ColorMode::Rgb);

    assert!(h.state().session.active().unwrap().doc.selection.is_none());
    drag_to(&mut h, "Red copy", "Load channel as selection");
    assert!(h.state().session.active().unwrap().doc.selection.is_some(), "the white channel selects everything");
}

/// The footer follows the targeted row shown by the panel when mask painting is active,
/// even though the engine's channel-view target remains Composite. Other channel targets
/// continue to load their own values, and each load remains one undoable selection change.
#[test]
fn footer_loads_the_selected_layer_mask_and_preserves_color_alpha_targets() {
    let (mut h, _) = channels_harness();
    h.state_mut().run("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    h.state_mut().run("layer.layerMask.hideAll", json!({})).unwrap();
    h.state_mut().run("layer.layerMask.revealSelection", json!({})).unwrap();
    h.state_mut().run("channel.target", json!({"channel": "composite"})).unwrap();
    h.state_mut().ui.mask_target = true;
    h.run_steps(2);

    let click_load = |h: &mut egui_kittest::Harness<'_, PhotocraftApp>| {
        use egui::{Modifiers, PointerButton};
        let p = at(h, "Load channel as selection");
        h.hover_at(p);
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(1);
        h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
    };
    let sample =
        |h: &egui_kittest::Harness<'_, PhotocraftApp>, x, y| h.state().session.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(x, y, 0);

    assert_eq!(h.state().session.active().unwrap().channel_view.target, ChannelTarget::Composite);
    click_load(&mut h);
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 0.0), "mask row selection wins over the Composite engine target");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 0.0));

    h.state_mut().ui.mask_target = false;
    h.state_mut().run("channel.target", json!({"channel": "composite"})).unwrap();
    click_load(&mut h);
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 1.0), "composite luminosity remains available");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 0.0));
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 1.0));

    h.state_mut().run("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    h.state_mut().run("channel.new", json!({"fill": "selection"})).unwrap();
    h.state_mut().run("select.deselect", json!({})).unwrap();
    h.state_mut().run("channel.target", json!({"channel": "red"})).unwrap();
    click_load(&mut h);
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 1.0), "color channel still loads its own plane");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 1.0));

    h.state_mut().run("channel.target", json!({"channel": 1})).unwrap();
    click_load(&mut h);
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 0.0), "alpha channel still loads its own plane");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 1.0));
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!((sample(&h, 8, 8), sample(&h, 48, 8)), (1.0, 0.0));
}

/// A plain click on a row still targets it, and a drag released elsewhere does nothing.
#[test]
fn clicks_still_target_and_a_drop_elsewhere_does_nothing() {
    let (mut h, _) = channels_harness();
    let p = at(&h, "Alpha 1");
    h.hover_at(p);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
    h.run_steps(3);
    assert_eq!(h.state().session.active().unwrap().channel_view.target, ChannelTarget::Alpha(0));

    let steps = h.state().session.active().unwrap().history.past_len();
    let (from, to) = (at(&h, "Alpha 1"), at(&h, "Blue"));
    drag(&mut h, from, to);
    drag_to(&mut h, "Alpha 1", "Save selection as channel");
    assert_eq!(alphas(&h), ["Alpha 1"]);
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "nothing ran");
    assert!(h.ctx.data(|d| d.get_temp::<(Row, String)>(drag_id())).is_none(), "the drag ended");
}

/// Double-clicking the layer mask's row opens Layer Mask Display Options, as in Photoshop; OK
/// sets the rubylith's colour and opacity, a view setting rather than an undo step.
#[test]
fn double_clicking_the_layer_mask_row_opens_its_display_options() {
    let (mut h, _) = channels_harness();
    let steps = h.state().session.active().unwrap().history.past_len();
    let p = at(&h, "Masked Mask");
    h.hover_at(p);
    h.run_steps(30);
    for pressed in [true, false, true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    h.run_steps(3);
    let d = h.state().ui.dialogs.last().expect("a dialog opens");
    assert!(crate::channel_options_ui::owns(&d.fields), "{:?}", d.fields);
    assert_eq!((d.fields["channel"].clone(), d.fields["color"].clone(), d.fields["opacity"].clone()), (json!("mask"), json!("#ff0000"), json!(50.0)));
    let id = d.id;
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert("color".into(), json!("#0000ff"));
    d.fields.insert("opacity".into(), json!(80.0));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    let o = &h.state().session.prefs().layer_mask_overlay;
    assert_eq!((o.color.as_str(), o.opacity), ("#0000ff", 80.0));
    assert_eq!(h.state().session.active().unwrap().history.past_len(), steps, "no history step");
}

/// The layer mask's row asks "Apply mask to layer before removing?", like its Layers thumbnail.
#[test]
fn the_layer_mask_row_on_the_trash_asks_first() {
    let (mut h, masked) = channels_harness();
    drag_to(&mut h, "Masked Mask", "Delete current channel");
    assert!(h.state().ui.dialogs.iter().any(|d| crate::mask_trash_prompt::owns(&d.fields)), "the prompt is open");
    let st = h.state().session.active().unwrap();
    assert!(st.doc.layer(photocraft_doc::LayerId(masked)).unwrap().mask.is_some(), "nothing deleted yet");
}
