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

/// #4444: click and hold on Change Screen Mode opens its list like the tool flyouts, without
/// cycling the mode on release; a plain click still cycles, and right-click still opens it.
#[test]
fn long_press_screen_mode_opens_the_modes_without_cycling() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
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
    // Remove leads the flyout, so the slot shows it until another J tool is picked.
    assert_eq!(j.first(), Some(&Tool::Remove));
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
    // Photoshop's Brush group, Color Replacement after the Mixer Brush.
    assert_eq!(slot, &[Tool::Brush, Tool::Pencil, Tool::MixerBrush, Tool::ColorReplacement]);
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
    // The last of the four 26-point rows.
    let row = egui::pos2(menu.left() + 65.0, menu.bottom() - 20.0);
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
