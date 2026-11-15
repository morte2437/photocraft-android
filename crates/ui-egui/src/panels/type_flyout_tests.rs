use super::*;

fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
    let mut out = ctx.run_ui(
        egui::RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() },
        |ui| toolbar(app, ui),
    );
    out.textures_delta.clear();
}

#[test]
fn long_press_type_button_selects_type_then_vertical_from_the_flyout() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert_ne!(app.ui.tool, Tool::Type);
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::Type)).unwrap();
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = buttons[index].center();
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    frame(&mut app, &ctx, 1.36, vec![]);
    assert_eq!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).map(|(id, _)| id), Some(egui::Id::unique(("tool-slot", index))));
    frame(&mut app, &ctx, 1.4, vec![pointer(at, false)]);
    frame(&mut app, &ctx, 1.45, vec![]);
    // The held button's tool is selected, as Photoshop does (#4453).
    assert_eq!(app.ui.tool, Tool::Type);
    let key = egui::Id::unique(("tool-slot", index));
    let menu = ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let row = egui::pos2(menu.left() + 65.0, menu.top() + 39.0 + 8.0);
    frame(&mut app, &ctx, 2.0, vec![egui::Event::PointerMoved(row), pointer(row, true)]);
    frame(&mut app, &ctx, 2.05, vec![pointer(row, false)]);
    assert_eq!(app.ui.tool, Tool::VerticalType);
    assert!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).is_none());
}

/// #9354 review: with no document in Standard Screen Mode, Cycle is unavailable (#9349), so
/// Change Screen Mode is greyed out: a click sends no viewport command.
#[test]
fn screen_mode_button_is_inert_without_a_document() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", serde_json::json!({"width": 64, "height": 48})).unwrap();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    // Found while a document makes it clickable: the toolbar's last button.
    let at = ctx
        .viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
                .map(|w| w.rect)
                .max_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())))
        })
        .unwrap()
        .center();
    app.run("file.close", serde_json::json!({"save": false})).unwrap();
    frame(&mut app, &ctx, 1.0, vec![]);
    assert_eq!(crate::view_cmds::is_enabled(&app, "view.screenMode.cycle"), Some(false));
    let pointer = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let mut sent = Vec::new();
    for (t, events) in [(2.0, vec![egui::Event::PointerMoved(at), pointer(true)]), (2.1, vec![pointer(false)]), (2.15, vec![])] {
        let mut out = ctx.run_ui(
            egui::RawInput { time: Some(t), events, screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() },
            |ui| toolbar(&mut app, ui),
        );
        out.textures_delta.clear();
        sent.extend(out.viewport_output.values().flat_map(|v| v.commands.clone()));
    }
    assert!(!sent.iter().any(|c| matches!(c, egui::ViewportCommand::Fullscreen(_))), "no screen mode change was sent: {sent:?}");
    assert_eq!(app.ui.view.screen_mode, "standard");
}

/// #4444: click and hold on Change Screen Mode opens its list like the tool flyouts, without
/// cycling the mode on release; a plain click still cycles, and right-click still opens it.
#[test]
fn long_press_screen_mode_opens_the_modes_without_cycling() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    // A document open: Full Screen Mode needs one (#9349), so without it a click can't cycle.
    app.run("file.new", serde_json::json!({"width": 64, "height": 48})).unwrap();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    // The toolbar's last button: Change Screen Mode, after Quick Mask.
    let at = ctx
        .viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
                .map(|w| w.rect)
                .max_by(|a, b| a.top().total_cmp(&b.top()).then(a.left().total_cmp(&b.left())))
        })
        .unwrap()
        .center();
    let standard = |app: &PhotocraftApp| crate::view_cmds::checked(app, "view.screenMode.standard") == Some(true);
    let menu_open = |ctx: &egui::Context| egui::Popup::is_any_open(ctx);
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    assert!(standard(&app));
    // Hold, then release: the list opens and stays open, and the mode is unchanged.
    frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    frame(&mut app, &ctx, 1.2, vec![]);
    assert!(!menu_open(&ctx), "not before the hold time");
    frame(&mut app, &ctx, 1.4, vec![]);
    assert!(menu_open(&ctx), "open while still held");
    frame(&mut app, &ctx, 1.5, vec![pointer(at, false)]);
    frame(&mut app, &ctx, 1.55, vec![]);
    assert!(menu_open(&ctx), "the release keeps it open");
    assert!(standard(&app), "the release doesn't cycle the mode");
    // Escape closes it; a plain click cycles the mode and opens nothing.
    frame(
        &mut app,
        &ctx,
        2.0,
        vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
    );
    frame(&mut app, &ctx, 2.05, vec![]);
    assert!(!menu_open(&ctx));
    frame(&mut app, &ctx, 3.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    frame(&mut app, &ctx, 3.1, vec![pointer(at, false)]);
    frame(&mut app, &ctx, 3.15, vec![]);
    assert!(!standard(&app), "a click cycles");
    assert!(!menu_open(&ctx));
    // Right-click opens it too.
    let secondary = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 4.0, vec![secondary(true)]);
    frame(&mut app, &ctx, 4.05, vec![secondary(false)]);
    frame(&mut app, &ctx, 4.1, vec![]);
    assert!(menu_open(&ctx));
}

/// Every text drawn in one toolbar frame.
fn frame_texts(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) -> Vec<String> {
    fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut out = ctx.run_ui(
        egui::RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() },
        |ui| toolbar(app, ui),
    );
    out.textures_delta.clear();
    let mut texts = Vec::new();
    out.shapes.iter().for_each(|c| collect(&c.shape, &mut texts));
    texts
}

/// #4450: no tool tooltip while a flyout is open or a tool button is held to open one, for the
/// held tool or any other; a plain hover still shows its tooltip.
#[test]
fn no_tooltips_over_an_open_tool_flyout() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let slot_of = |tool: Tool| TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&tool)).unwrap();
    let (eyedropper, brush) = (buttons[slot_of(Tool::Eyedropper)].center(), buttons[slot_of(Tool::Brush)].center());
    let tip = |tool: Tool| format!("{}  ({})", tl!(tool.label()), tool.key());
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // A plain hover shows the tooltip.
    frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(brush)]);
    let mut texts = Vec::new();
    for k in 1..=10 {
        texts = frame_texts(&mut app, &ctx, 1.0 + 0.1 * f64::from(k), vec![]);
    }
    assert!(texts.contains(&tip(Tool::Brush)), "hovered long enough: {texts:?}");
    // Held on the Eyedropper: no tooltip before or after its flyout opens.
    frame(&mut app, &ctx, 3.0, vec![egui::Event::PointerMoved(eyedropper), pointer(eyedropper, true)]);
    for k in 1..=10 {
        let texts = frame_texts(&mut app, &ctx, 3.0 + 0.1 * f64::from(k), vec![]);
        assert!(!texts.contains(&tip(Tool::Eyedropper)), "held {k}: {texts:?}");
    }
    assert!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).is_some(), "the flyout is open");
    // Released, with the flyout still open: none for the held tool or another one.
    frame(&mut app, &ctx, 4.1, vec![pointer(eyedropper, false)]);
    for (start, at, tool) in [(4.5, eyedropper, Tool::Eyedropper), (6.0, brush, Tool::Brush)] {
        frame(&mut app, &ctx, start, vec![egui::Event::PointerMoved(at)]);
        for k in 1..=10 {
            let texts = frame_texts(&mut app, &ctx, start + 0.1 * f64::from(k), vec![]);
            assert!(!texts.contains(&tip(tool)), "{tool:?} under the flyout: {texts:?}");
        }
    }
}

/// #4460: a press outside an open flyout closes it, even when it becomes a drag (cropping or
/// painting on the canvas) rather than a click; a press on the flyout keeps it for its pick.
/// #4932: so does a click on its own tool button.
#[test]
fn a_drag_outside_closes_the_tool_flyout() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::Brush)).unwrap();
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = buttons[index].center();
    let flyout = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).is_some();
    let secondary = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
    let primary = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // A right-click on the button of an open flyout closes it (#4932), so open only a closed one.
    let open = |app: &mut PhotocraftApp, t: f64| {
        if !flyout(&ctx) {
            frame(app, &ctx, t, vec![egui::Event::PointerMoved(at), secondary(true)]);
            frame(app, &ctx, t + 0.05, vec![secondary(false)]);
            frame(app, &ctx, t + 0.1, vec![]);
        }
        assert!(flyout(&ctx), "right-click opens it");
    };
    // A press on the flyout itself keeps it open (the row picks on release).
    open(&mut app, 1.0);
    let menu = ctx.memory(|m| m.area_rect(egui::Id::unique(("tool-slot", index)).with("flyout"))).unwrap();
    frame(&mut app, &ctx, 1.5, vec![egui::Event::PointerMoved(menu.center()), primary(menu.center(), true)]);
    assert!(flyout(&ctx), "a press on the flyout keeps it");
    frame(&mut app, &ctx, 1.55, vec![primary(menu.center(), false)]);
    // A drag far from the toolbar (the canvas) closes it as soon as it starts.
    open(&mut app, 2.0);
    let canvas = egui::pos2(900.0, 900.0);
    frame(&mut app, &ctx, 2.5, vec![egui::Event::PointerMoved(canvas), primary(canvas, true)]);
    frame(&mut app, &ctx, 2.55, vec![egui::Event::PointerMoved(canvas + vec2(80.0, 60.0))]);
    assert!(!flyout(&ctx), "a press outside closes it, even one that becomes a drag");
    frame(&mut app, &ctx, 2.6, vec![primary(canvas + vec2(80.0, 60.0), false)]);
    assert!(!flyout(&ctx));
    // #4932: clicking its own tool button again closes it, with either button.
    open(&mut app, 3.0);
    frame(&mut app, &ctx, 3.5, vec![egui::Event::PointerMoved(at), secondary(true)]);
    frame(&mut app, &ctx, 3.55, vec![secondary(false)]);
    frame(&mut app, &ctx, 3.6, vec![]);
    assert!(!flyout(&ctx), "a second right-click on the button closes it");
    open(&mut app, 4.0);
    app.ui.tool = Tool::Move;
    frame(&mut app, &ctx, 4.5, vec![egui::Event::PointerMoved(at), primary(at, true)]);
    frame(&mut app, &ctx, 4.55, vec![primary(at, false)]);
    frame(&mut app, &ctx, 4.6, vec![]);
    assert!(!flyout(&ctx), "a click on the button closes it");
    assert_eq!(app.ui.tool, Tool::Brush, "and picks the tool, as any click on it does");
    // The release that ends a long press keeps the flyout it opened.
    frame(&mut app, &ctx, 5.0, vec![egui::Event::PointerMoved(at), primary(at, true)]);
    frame(&mut app, &ctx, 5.2, vec![]);
    frame(&mut app, &ctx, 5.5, vec![]);
    assert!(flyout(&ctx), "a long press opens it");
    frame(&mut app, &ctx, 5.55, vec![primary(at, false)]);
    frame(&mut app, &ctx, 5.6, vec![]);
    assert!(flyout(&ctx), "releasing the long press keeps it open");
}

#[test]
fn red_eye_is_in_the_j_flyout() {
    let j = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).find(|slot| slot.contains(&Tool::SpotHealing)).expect("J group");
    assert!(j.contains(&Tool::RedEye), "{j:?}");
    assert_eq!(j.last(), Some(&Tool::RedEye));
    // Spot Healing Brush leads the flyout, so the slot shows it until another J tool is picked.
    assert_eq!(j.first(), Some(&Tool::SpotHealing));
}

#[test]
fn art_history_is_in_the_y_flyout() {
    let y = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).find(|slot| slot.contains(&Tool::HistoryBrush)).expect("Y group");
    assert_eq!(*y, [Tool::HistoryBrush, Tool::ArtHistoryBrush]);
}

#[test]
fn long_press_clone_stamp_selects_pattern_stamp() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::CloneStamp)).unwrap();
    let slot = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).nth(index).copied().unwrap();
    assert_eq!(slot, &[Tool::CloneStamp, Tool::PatternStamp]);
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = buttons[index].center();
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    frame(&mut app, &ctx, 1.36, vec![]);
    assert_eq!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).map(|(id, _)| id), Some(egui::Id::unique(("tool-slot", index))));
    frame(&mut app, &ctx, 1.4, vec![pointer(at, false)]);
    frame(&mut app, &ctx, 1.45, vec![]);
    let key = egui::Id::unique(("tool-slot", index));
    let menu = ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let row = egui::pos2(menu.left() + 65.0, menu.top() + 39.0 + 8.0);
    frame(&mut app, &ctx, 2.0, vec![egui::Event::PointerMoved(row), pointer(row, true)]);
    frame(&mut app, &ctx, 2.05, vec![pointer(row, false)]);
    assert_eq!(app.ui.tool, Tool::PatternStamp);
}

#[test]
fn long_press_brush_selects_color_replacement() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::Brush)).unwrap();
    let slot = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).nth(index).copied().unwrap();
    // Photoshop's Brush group, Color Replacement before the Mixer Brush.
    assert_eq!(slot, &[Tool::Brush, Tool::Pencil, Tool::ColorReplacement, Tool::MixerBrush]);
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = buttons[index].center();
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    frame(&mut app, &ctx, 1.36, vec![]);
    frame(&mut app, &ctx, 1.4, vec![pointer(at, false)]);
    frame(&mut app, &ctx, 1.45, vec![]);
    let key = egui::Id::unique(("tool-slot", index));
    let menu = ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    // The third of the four 26-point rows.
    let row = egui::pos2(menu.left() + 65.0, menu.bottom() - 46.0);
    frame(&mut app, &ctx, 2.0, vec![egui::Event::PointerMoved(row), pointer(row, true)]);
    frame(&mut app, &ctx, 2.05, vec![pointer(row, false)]);
    assert_eq!(app.ui.tool, Tool::ColorReplacement);
    assert_eq!(crate::icons::tool_icon(Tool::ColorReplacement), "brush-replace");
}

#[test]
fn rotate_view_is_in_the_hand_flyout() {
    let hand = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).find(|slot| slot.contains(&Tool::Hand)).expect("Hand group");
    assert_eq!(*hand, [Tool::Hand, Tool::RotateView]);
    assert_eq!(Tool::RotateView.key(), 'R');
}

#[test]
fn pro_toolbar_glyphs_fill_two_thirds_of_the_button() {
    // #2665: Photoshop-style themes draw tool glyphs larger than other icon buttons.
    assert_eq!(toolbar_icon_size(30.0, true), 20.0);
    assert_eq!(toolbar_icon_size(36.0, false), (36.0f32 * 0.52).round(), "other themes keep the icon-button size");
    // The glyph stays inside the button with room for the flyout triangle's corner.
    assert!(toolbar_icon_size(30.0, true) <= 30.0 - 2.0 * 4.0);
}

#[test]
fn long_press_y_button_selects_art_history() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::HistoryBrush)).unwrap();
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = buttons[index].center();
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
    frame(&mut app, &ctx, 1.36, vec![]);
    assert_eq!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).map(|(id, _)| id), Some(egui::Id::unique(("tool-slot", index))));
    frame(&mut app, &ctx, 1.4, vec![pointer(at, false)]);
    frame(&mut app, &ctx, 1.45, vec![]);
    let key = egui::Id::unique(("tool-slot", index));
    let menu = ctx.memory(|m| m.area_rect(key.with("flyout"))).unwrap();
    let row = egui::pos2(menu.left() + 65.0, menu.top() + 39.0 + 8.0);
    frame(&mut app, &ctx, 2.0, vec![egui::Event::PointerMoved(row), pointer(row, true)]);
    frame(&mut app, &ctx, 2.05, vec![pointer(row, false)]);
    assert_eq!(app.ui.tool, Tool::ArtHistoryBrush);
}

#[test]
fn next_slot_tool_wraps_around_the_flyout() {
    // One-tool slot: Alt-click is a no-op (still “cycles” to itself).
    assert_eq!(next_slot_tool(&[Tool::Frame], Tool::Frame), Tool::Frame);
    // Two-tool slot (Hand ↔ RotateView).
    let two = [Tool::Hand, Tool::RotateView];
    assert_eq!(next_slot_tool(&two, Tool::Hand), Tool::RotateView);
    assert_eq!(next_slot_tool(&two, Tool::RotateView), Tool::Hand, "wrap");
    // Three-tool slot.
    let three = [Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso];
    assert_eq!(next_slot_tool(&three, Tool::Lasso), Tool::PolygonLasso);
    assert_eq!(next_slot_tool(&three, Tool::PolygonLasso), Tool::MagneticLasso);
    assert_eq!(next_slot_tool(&three, Tool::MagneticLasso), Tool::Lasso, "wrap");
    // Four-tool marquee flyout: starts from the current tool, not always the first.
    let slot = [Tool::RectMarquee, Tool::EllipseMarquee, Tool::SingleRowMarquee, Tool::SingleColumnMarquee];
    assert_eq!(next_slot_tool(&slot, Tool::RectMarquee), Tool::EllipseMarquee);
    assert_eq!(next_slot_tool(&slot, Tool::EllipseMarquee), Tool::SingleRowMarquee);
    assert_eq!(next_slot_tool(&slot, Tool::SingleColumnMarquee), Tool::RectMarquee);
    assert_eq!(next_slot_tool(&slot, Tool::Brush), Tool::EllipseMarquee, "unknown from → after the first");
    // Empty slot: no panic, the current tool stays.
    assert_eq!(next_slot_tool(&[], Tool::Brush), Tool::Brush);
}

#[test]
fn slot_tool_is_the_current_tool_else_the_slots_last_else_its_first() {
    let lasso = [Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso];
    assert_eq!(slot_tool(Tool::Move, &[], &lasso), Tool::Lasso);
    let used = [Tool::Brush, Tool::PolygonLasso, Tool::Lasso];
    assert_eq!(slot_tool(Tool::Move, &used, &lasso), Tool::PolygonLasso);
    assert_eq!(slot_tool(Tool::MagneticLasso, &used, &lasso), Tool::MagneticLasso);
    // Polygonal Lasso hidden in Edit › Toolbar: the slot falls back on the next one used.
    assert_eq!(slot_tool(Tool::Move, &used, &[Tool::Lasso, Tool::MagneticLasso]), Tool::Lasso);
    // Slots don't line up with keys: Blur's slot and the two single marquees have no key at all,
    // and the Hand slot has two.
    let used = [Tool::Sharpen, Tool::SingleRowMarquee, Tool::RotateView];
    assert_eq!(slot_tool(Tool::Move, &used, &[Tool::Blur, Tool::Sharpen, Tool::Smudge]), Tool::Sharpen);
    let marquee = [Tool::RectMarquee, Tool::EllipseMarquee, Tool::SingleRowMarquee, Tool::SingleColumnMarquee];
    assert_eq!(slot_tool(Tool::Move, &used, &marquee), Tool::SingleRowMarquee);
    assert_eq!(slot_tool(Tool::Move, &used, &[Tool::Hand, Tool::RotateView]), Tool::RotateView);
    // Empty slot: no panic, the current tool stays.
    assert_eq!(slot_tool(Tool::Brush, &used, &[]), Tool::Brush);
}

/// #3569: after a restart a slot is still on the tool it was left on, whichever tool is selected:
/// a click picks that tool, not the slot's first.
#[test]
fn slots_come_back_on_their_last_tool_after_a_restart() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.prefs.edit(|p| p.group_tools = json!(["PolygonLasso", "PaintBucket", "Pencil", "Sharpen", "SingleRowMarquee"]));
    crate::tool_memory::restore(&mut app);
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let pointer = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    let slots = [
        (Tool::Brush, Tool::Pencil),
        (Tool::Gradient, Tool::PaintBucket),
        (Tool::Lasso, Tool::PolygonLasso),
        (Tool::Blur, Tool::Sharpen),
        (Tool::RectMarquee, Tool::SingleRowMarquee),
    ];
    for (i, (first, last)) in slots.into_iter().enumerate() {
        let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&first)).unwrap();
        let at = buttons[index].center();
        let t = 1.0 + i as f64;
        frame(&mut app, &ctx, t, vec![egui::Event::PointerMoved(at), pointer(at, true)]);
        frame(&mut app, &ctx, t + 0.05, vec![pointer(at, false)]);
        frame(&mut app, &ctx, t + 0.1, vec![]);
        assert_eq!(app.ui.tool, last);
    }
}

/// Edit › Toolbar hidden tools shrink the Alt-click cycle to the remaining flyout members.
#[test]
fn visible_sections_drop_hidden_tools_from_alt_click_slots() {
    let hidden = vec!["EllipseMarquee".into(), "SingleRowMarquee".into()];
    let sections = visible_sections(&hidden);
    let marquee = sections
        .iter()
        .flat_map(|s| s.iter())
        .find(|(_, tools)| tools.contains(&Tool::RectMarquee))
        .map(|(_, tools)| tools.as_slice())
        .expect("marquee slot remains");
    assert_eq!(marquee, &[Tool::RectMarquee, Tool::SingleColumnMarquee]);
    assert_eq!(next_slot_tool(marquee, Tool::RectMarquee), Tool::SingleColumnMarquee);
    assert_eq!(next_slot_tool(marquee, Tool::SingleColumnMarquee), Tool::RectMarquee, "wrap over the visible pair");
}

/// Photoshop: Alt/Option-click a toolbar button cycles its flyout tools without opening the menu.
#[test]
fn alt_click_cycles_the_tool_flyout_slot() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::RectMarquee)).unwrap();
    let slot = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).nth(index).copied().unwrap();
    assert_eq!(slot, &[Tool::RectMarquee, Tool::EllipseMarquee, Tool::SingleRowMarquee, Tool::SingleColumnMarquee]);
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = buttons[index].center();
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    let pointer = |pos, pressed, modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    let flyout = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(egui::Id::unique("tool-flyout"))).is_some();
    app.ui.tool = Tool::RectMarquee;
    // Plain click keeps the shown tool.
    frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(at), pointer(at, true, Default::default())]);
    frame(&mut app, &ctx, 1.05, vec![pointer(at, false, Default::default())]);
    frame(&mut app, &ctx, 1.1, vec![]);
    assert_eq!(app.ui.tool, Tool::RectMarquee);
    assert!(!flyout(&ctx));
    // Alt-click advances through every flyout entry, including ones Shift+M skips.
    // Clear i.modifiers before the release while keeping Alt on the release event, so a
    // regression that only reads i.modifiers (and ignores the pointer-release modifiers) fails.
    for (t, want) in [(1.5, Tool::EllipseMarquee), (2.0, Tool::SingleRowMarquee), (2.5, Tool::SingleColumnMarquee), (3.0, Tool::RectMarquee)] {
        frame(&mut app, &ctx, t, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(at), pointer(at, true, alt)]);
        frame(&mut app, &ctx, t + 0.05, vec![egui::Event::ModifiersChanged(Default::default()), pointer(at, false, alt)]);
        frame(&mut app, &ctx, t + 0.1, vec![]);
        assert_eq!(app.ui.tool, want, "alt-click at {t}");
        assert!(!flyout(&ctx), "alt-click must not open the flyout");
    }
    // From another tool, Alt-click advances from the slot's remembered (shown) tool: Rect after the wrap.
    app.ui.tool = Tool::Brush;
    frame(&mut app, &ctx, 4.0, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(at), pointer(at, true, alt)]);
    frame(&mut app, &ctx, 4.05, vec![egui::Event::ModifiersChanged(Default::default()), pointer(at, false, alt)]);
    frame(&mut app, &ctx, 4.1, vec![]);
    assert_eq!(app.ui.tool, Tool::EllipseMarquee, "inactive slot cycles from its shown tool");
}

/// #10301 CodeRabbit: two rapid Alt-clicks on Hand ↔ RotateView must not fit-on-screen.
#[test]
fn rapid_alt_clicks_on_hand_slot_do_not_fit_on_screen() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::Hand)).unwrap();
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = buttons[index].center();
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    let pointer = |pos, pressed, modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    app.ui.tool = Tool::RotateView;
    let zoom_before = app.ui.views.first().map(|v| v.zoom).unwrap_or(1.0);
    // Two Alt-clicks within egui's double-click window: cycle Hand → RotateView, never fit-on-screen.
    frame(&mut app, &ctx, 1.0, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(at), pointer(at, true, alt)]);
    frame(&mut app, &ctx, 1.02, vec![egui::Event::ModifiersChanged(alt), pointer(at, false, alt)]);
    frame(&mut app, &ctx, 1.04, vec![]);
    assert_eq!(app.ui.tool, Tool::Hand);
    frame(&mut app, &ctx, 1.08, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(at), pointer(at, true, alt)]);
    frame(&mut app, &ctx, 1.10, vec![egui::Event::ModifiersChanged(alt), pointer(at, false, alt)]);
    frame(&mut app, &ctx, 1.12, vec![]);
    assert_eq!(app.ui.tool, Tool::RotateView, "second Alt-click cycles back; must not stick on Hand from double-click");
    let zoom_after = app.ui.views.first().map(|v| v.zoom).unwrap_or(1.0);
    assert_eq!(zoom_after, zoom_before, "Alt double-click must not run view.fitOnScreen");
}

/// #10301 CodeRabbit: plain click after Alt-click within the double-click window must not fit.
#[test]
fn plain_click_after_alt_on_hand_slot_does_not_fit_on_screen() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    // Far from fit-on-screen so a real Hand double-click is observable.
    if let Some(v) = app.ui.views.first_mut() {
        v.zoom = 4.0;
    }
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    frame(&mut app, &ctx, 0.0, vec![]);
    frame(&mut app, &ctx, 0.1, vec![]);
    let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::Hand)).unwrap();
    let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
    let mut buttons: Vec<Rect> = ctx.viewport(|v| {
        v.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, w)| w.iter())
            .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
            .map(|w| w.rect)
            .collect()
    });
    buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = buttons[index].center();
    let alt = egui::Modifiers { alt: true, ..Default::default() };
    let none = egui::Modifiers::default();
    let pointer = |pos, pressed, modifiers| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    app.ui.tool = Tool::RotateView;
    let zoom_of = |app: &PhotocraftApp| app.ui.views.first().map(|v| v.zoom).unwrap_or(1.0);
    assert_eq!(zoom_of(&app), 4.0);
    // Alt-click cycles to Hand; a plain click soon after is egui's "second" click of a double.
    frame(&mut app, &ctx, 1.0, vec![egui::Event::ModifiersChanged(alt), egui::Event::PointerMoved(at), pointer(at, true, alt)]);
    frame(&mut app, &ctx, 1.02, vec![egui::Event::ModifiersChanged(alt), pointer(at, false, alt)]);
    frame(&mut app, &ctx, 1.04, vec![]);
    assert_eq!(app.ui.tool, Tool::Hand);
    frame(&mut app, &ctx, 1.08, vec![egui::Event::ModifiersChanged(none), egui::Event::PointerMoved(at), pointer(at, true, none)]);
    frame(&mut app, &ctx, 1.10, vec![egui::Event::ModifiersChanged(none), pointer(at, false, none)]);
    frame(&mut app, &ctx, 1.12, vec![]);
    assert_eq!(app.ui.tool, Tool::Hand, "plain click keeps Hand selected");
    assert_eq!(zoom_of(&app), 4.0, "plain click paired with prior Alt-click must not fitOnScreen");
}
