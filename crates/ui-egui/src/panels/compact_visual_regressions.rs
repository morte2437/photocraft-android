use super::*;
use egui_kittest::kittest::Queryable;

fn input_at_scale(screen: Rect, scale: f32) -> egui::RawInput {
    let mut input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
    input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(scale);
    input
}

fn toolbar_harness(kind: theme::ThemeKind, scale: f32) -> egui_kittest::Harness<'static, PhotocraftApp> {
    toolbar_harness_sized(kind, scale, 360.0)
}

fn toolbar_harness_sized(kind: theme::ThemeKind, scale: f32, height: f32) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut harness = egui_kittest::Harness::builder().with_size(vec2(400.0, height)).with_pixels_per_point(scale).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                egui_extras::install_image_loaders(ui.ctx());
                theme::install_fonts(ui.ctx());
                theme::apply(ui.ctx(), kind);
                app.session.prefs.edit(|prefs| prefs.shortcuts.insert("window.togglePanels".into(), String::new()));
                ready = true;
                return;
            }
            crate::shortcuts::handle(app, ui.ctx());
            toolbar(app, ui);
        },
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default()),
    );
    harness.run_steps(4);
    harness
}

#[test]
fn compact_toolbar_scrolls_to_and_activates_the_last_tool() {
    for kind in theme::ThemeKind::ALL {
        for scale in [1.0, 1.5, 2.0] {
            let mut harness = toolbar_harness(kind, scale);
            harness.event(egui::Event::PointerMoved(pos2(25.0, 100.0)));
            harness.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -1500.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            });
            harness.run_steps(30);
            // Off the toolbar, so a tooltip of the tool scrolled under the pointer isn't a
            // second "Zoom Tool" node.
            harness.event(egui::Event::PointerMoved(pos2(300.0, 100.0)));
            harness.run_steps(2);
            let node = harness.get_by_label("Zoom Tool  (Z)");
            let rect = node.rect();
            assert!(rect.top() >= 0.0 && rect.bottom() <= 360.0, "{kind:?} at {scale}x: last tool remains outside the toolbar: {rect:?}");
            node.click();
            harness.run_steps(3);
            assert_eq!(harness.state().ui.tool, Tool::Zoom, "the scrolled button must select its real tool");
        }
    }
}

#[test]
fn compact_toolbar_tab_reveals_and_activates_the_last_tool() {
    for kind in theme::ThemeKind::ALL {
        for scale in [1.0, 1.5, 2.0] {
            let mut harness = toolbar_harness(kind, scale);
            // The supported shortcut preference clears the canvas Tab binding. From no
            // focus, every step below is a real Tab processed by the app's dispatcher.
            let mut reached = false;
            for _ in 0..80 {
                harness.key_press(egui::Key::Tab);
                harness.run_steps(3);
                reached = harness.get_by_label("Zoom Tool  (Z)").is_focused();
                if reached {
                    break;
                }
            }
            assert!(reached, "{kind:?} at {scale}x: Tab must reach the last tool");
            harness.run_steps(20);
            let rect = harness.get_by_label("Zoom Tool  (Z)").rect();
            assert!(rect.top() >= 0.0 && rect.bottom() <= 360.0, "{kind:?} at {scale}x: focused tool remains clipped: {rect:?}");
            harness.key_press(egui::Key::Space);
            harness.run_steps(3);
            assert_eq!(harness.state().ui.tool, Tool::Zoom, "Space must select the revealed tool");
        }
    }
}

/// A window taller than the tools keeps the colour chips right under the tool column (its
/// last row is Edit Toolbar), as Photoshop does, instead of at the bottom of the toolbar.
#[test]
fn tall_toolbar_keeps_the_colour_chips_under_the_tools() {
    for kind in theme::ThemeKind::ALL {
        let harness = toolbar_harness_sized(kind, 1.0, 1600.0);
        let last = harness.get_by_label("Edit Toolbar…").rect();
        let swap = harness.get_by_label("Swap colours (X)").rect();
        assert!(swap.top() - last.bottom() < 80.0, "{kind:?}: the chips {swap:?} sit far below the last tool {last:?}");
    }
}

#[test]
fn capture_compact_visual_fixtures() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(directory) = std::env::var_os("CRAFT_UI_VISUAL_FIXTURES").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&directory).unwrap();
    for kind in crate::theme::ThemeKind::ALL {
        for (width, height) in [(800.0, 480.0), (1280.0, 720.0)] {
            for scale in [1.0, 1.5, 2.0] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.session.tools.foreground = [1.0, 0.0, 1.0, 1.0];
                app.session.tools.background = [0.0, 1.0, 0.0, 1.0];
                let mut ready = false;
                let mut harness = egui_kittest::Harness::builder().with_size(vec2(width, height)).with_pixels_per_point(scale).wgpu().build_ui_state(
                    move |ui, app: &mut PhotocraftApp| {
                        if !ready {
                            egui_extras::install_image_loaders(ui.ctx());
                            crate::theme::install_fonts(ui.ctx());
                            crate::theme::apply(ui.ctx(), kind);
                            ready = true;
                            return;
                        }
                        app.ui.tool = if Tokens::get(ui.ctx()).pro { Tool::Pencil } else { Tool::Brush };
                        options_bar(app, ui);
                        toolbar(app, ui);
                    },
                    app,
                );
                harness.input_mut().max_texture_side = Some(8192);
                harness.run_steps(4);
                let stem = format!("photocraft-compact-{}-{width}-{height}-{scale}x", kind.id());
                harness.render().unwrap().save(directory.join(format!("{stem}.png"))).unwrap();
                let mut regions = serde_json::Map::new();
                for id in ["options_bar", "toolbar"] {
                    let r = egui::containers::panel::PanelState::load(&harness.ctx, panel_state_id(&harness.ctx, id)).unwrap().outer_rect;
                    regions.insert(id.into(), json!([r.left(), r.top(), r.width(), r.height()]));
                }
                let metadata = json!({"screen": [width, height], "scale": scale, "regions": regions});
                std::fs::write(directory.join(format!("{stem}.json")), serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
                for _ in 0..6 {
                    harness.input_mut().events.extend([
                        egui::Event::PointerMoved(pos2(width * 0.5, 20.0)),
                        egui::Event::MouseWheel {
                            source: egui::MouseWheelSource::Unknown,
                            unit: egui::MouseWheelUnit::Point,
                            delta: vec2(-1500.0, 0.0),
                            modifiers: egui::Modifiers::NONE,
                            phase: egui::TouchPhase::Move,
                        },
                    ]);
                    harness.run_steps(1);
                }
                harness.render().unwrap().save(directory.join(format!("{stem}-scrolled.png"))).unwrap();
                std::fs::write(directory.join(format!("{stem}-scrolled.json")), serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
            }
        }
    }
}

/// #7316: a window too short for the tools scrolls them, and the scroll bar widened the UI
/// the colour chips were centred in, so they (and Switch Colors with them) sat a bar's width
/// right of the tools. They stay where they are in a window tall enough not to scroll.
#[test]
fn color_chips_stay_centred_when_the_tools_scroll() {
    let chips = |kind: crate::theme::ThemeKind, double: bool, height: f32, scale: f32| {
        let ctx = egui::Context::default();
        egui_extras::install_image_loaders(&ctx);
        crate::theme::install_fonts(&ctx);
        crate::theme::apply(&ctx, kind);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.session.tools.foreground = [1.0, 0.0, 1.0, 1.0];
        app.session.tools.background = [0.0, 1.0, 0.0, 1.0];
        app.ui.panels.toolbar_double = double;
        let input = || input_at_scale(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, height)), scale);
        // The scroll bar shows from the frame after the one that measured the tools.
        let mut output = ctx.run_ui(input(), |ui| toolbar(&mut app, ui));
        for _ in 0..2 {
            output.textures_delta.clear();
            output = ctx.run_ui(input(), |ui| toolbar(&mut app, ui));
        }
        output.textures_delta.clear();
        [Color32::from_rgb(255, 0, 255), Color32::GREEN].map(|color| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == color => Some(rect.rect.center()),
                    egui::Shape::Circle(circle) if circle.fill == color => Some(circle.center),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{kind:?}: both color chips are painted"))
                .x
        })
    };
    for kind in crate::theme::ThemeKind::ALL {
        for double in [false, true] {
            for scale in [1.0, 2.0] {
                let (short, tall) = (chips(kind, double, 420.0, scale), chips(kind, double, 1400.0, scale));
                assert_eq!(short, tall, "{kind:?}, two columns {double}, scale {scale}: the chips don't move when the tools scroll");
            }
        }
    }
}

#[test]
fn compact_toolbar_keeps_both_color_chips_visible() {
    for kind in crate::theme::ThemeKind::ALL {
        for height in [480.0, 720.0] {
            for scale in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                egui_extras::install_image_loaders(&ctx);
                crate::theme::install_fonts(&ctx);
                crate::theme::apply(&ctx, kind);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                app.session.tools.foreground = [1.0, 0.0, 1.0, 1.0];
                app.session.tools.background = [0.0, 1.0, 0.0, 1.0];
                let mut output = ctx.run_ui(input_at_scale(Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, height)), scale), |ui| toolbar(&mut app, ui));
                output.textures_delta.clear();
                for color in [Color32::from_rgb(255, 0, 255), Color32::GREEN] {
                    let (clip, chip) = output
                        .shapes
                        .iter()
                        .find_map(|shape| match &shape.shape {
                            egui::Shape::Rect(rect) if rect.fill == color => Some((shape.clip_rect, rect.rect)),
                            egui::Shape::Circle(circle) if circle.fill == color => {
                                Some((shape.clip_rect, Rect::from_center_size(circle.center, vec2(circle.radius * 2.0, circle.radius * 2.0))))
                            }
                            _ => None,
                        })
                        .unwrap_or_else(|| panic!("{kind:?} at {height}/{scale}: both color chips are painted"));
                    assert!(clip.contains_rect(chip) && chip.bottom() <= height, "{kind:?} at {height}: color chip {chip:?} clipped by {clip:?}");
                }
            }
        }
    }
}

#[test]
fn tab_focus_reveals_the_final_narrow_tool_option() {
    for kind in theme::ThemeKind::ALL {
        for scale in [1.0, 1.5, 2.0] {
            let mut ready = false;
            let mut harness = egui_kittest::Harness::builder().with_size(vec2(360.0, 120.0)).with_pixels_per_point(scale).build_ui_state(
                move |ui, app: &mut PhotocraftApp| {
                    if !ready {
                        egui_extras::install_image_loaders(ui.ctx());
                        theme::install_fonts(ui.ctx());
                        theme::apply(ui.ctx(), kind);
                        app.ui.tool = if Tokens::get(ui.ctx()).pro { Tool::Pencil } else { Tool::Brush };
                        app.session.prefs.edit(|prefs| prefs.shortcuts.insert("window.togglePanels".into(), String::new()));
                        ready = true;
                        return;
                    }
                    crate::shortcuts::handle(app, ui.ctx());
                    options_bar(app, ui);
                },
                PhotocraftApp::new(photocraft_engine::Session::new(), Default::default()),
            );
            harness.run_steps(4);
            let pro = Tokens::get(&harness.ctx).pro;
            let last_label = if pro { "Auto Erase" } else { "Pressure for Opacity" };
            // Only the canvas Tab shortcut is cleared, using its public preference;
            // no response is assigned focus programmatically.
            let mut reached = false;
            for _ in 0..80 {
                harness.key_press(egui::Key::Tab);
                harness.run_steps(3);
                let label = harness.output().shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text.contains(last_label) => Some(text.visual_bounding_rect()),
                    _ => None,
                });
                reached = label.is_some_and(|label| {
                    harness.ctx.memory(|memory| memory.focused()).and_then(|id| harness.ctx.read_response(id)).is_some_and(|response| {
                        response.rect.intersects(label)
                            || ((label.left() - response.rect.right()).abs() <= 12.0
                                && response.rect.top() < label.bottom()
                                && response.rect.bottom() > label.top())
                    })
                });
                if reached {
                    break;
                }
            }
            assert!(reached, "{kind:?} at {scale}x: Tab must reach the final tool option");
            harness.run_steps(20);
            let (clip, label) = harness
                .output()
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text.contains(last_label) => Some((shape.clip_rect, text.visual_bounding_rect())),
                    _ => None,
                })
                .expect("the final option remains painted");
            assert!(clip.contains_rect(label), "{kind:?} at {scale}x: Tab leaves the focused final option outside {clip:?}: {label:?}");
            let previous = if pro { harness.state().ui.tool_options.pencil_auto_erase } else { harness.state().session.tools.brush.pressure_opacity };
            harness.key_press(egui::Key::Space);
            harness.run_steps(3);
            let current = if pro { harness.state().ui.tool_options.pencil_auto_erase } else { harness.state().session.tools.brush.pressure_opacity };
            assert_ne!(current, previous, "{kind:?} at {scale}x: Space must activate the revealed option");
            assert!(harness.state().ui.panels.options_bar, "widget traversal must preserve the options bar");
        }
    }
}

#[test]
fn a_wide_crop_options_bar_does_not_scroll() {
    for kind in crate::theme::ThemeKind::ALL {
        for scale in [1.0, 1.5, 2.0] {
            for mode in ["", crate::crop_size::WHR] {
                let ctx = egui::Context::default();
                crate::theme::install_fonts(&ctx);
                crate::theme::apply(&ctx, kind);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                if !Tokens::get(&ctx).pro {
                    continue;
                }
                app.ui.tool = Tool::Crop;
                app.ui.tool_options.crop_ratio = mode.into();
                let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 120.0));
                let clear_x = |output: &egui::FullOutput| {
                    output
                        .shapes
                        .iter()
                        .find_map(|s| match &s.shape {
                            egui::Shape::Text(t) if t.galley.job.text == "Clear" => Some(t.pos.x),
                            _ => None,
                        })
                        .unwrap()
                };
                let mut before = ctx.run_ui(input_at_scale(screen, scale), |ui| options_bar(&mut app, ui));
                before.textures_delta.clear();
                for _ in 0..3 {
                    before = ctx.run_ui(input_at_scale(screen, scale), |ui| options_bar(&mut app, ui));
                    before.textures_delta.clear();
                }
                let mut after = before.clone();
                for _ in 0..6 {
                    after = ctx.run_ui(
                        egui::RawInput {
                            events: vec![
                                egui::Event::PointerMoved(pos2(800.0, 20.0)),
                                egui::Event::MouseWheel {
                                    source: egui::MouseWheelSource::Unknown,
                                    unit: egui::MouseWheelUnit::Point,
                                    delta: vec2(-100.0, 0.0),
                                    modifiers: egui::Modifiers::NONE,
                                    phase: egui::TouchPhase::Move,
                                },
                            ],
                            ..input_at_scale(screen, scale)
                        },
                        |ui| options_bar(&mut app, ui),
                    );
                    after.textures_delta.clear();
                }
                assert_eq!(clear_x(&before), clear_x(&after), "a roomy bar must not have horizontal overflow");
            }
        }
    }
}

#[test]
fn the_crop_ratio_fields_show_three_decimals() {
    // #6487: a 1.414 : 1 ratio reads 1.414 in the bar, not 1.4.
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let Some(kind) = crate::theme::ThemeKind::ALL.into_iter().find(|k| {
        crate::theme::apply(&ctx, *k);
        Tokens::get(&ctx).pro
    }) else {
        return;
    };
    crate::theme::apply(&ctx, kind);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.ui.tool = Tool::Crop;
    app.ui.tool_options.crop_ratio = "1.414:1".into();
    let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 120.0));
    let mut out = ctx.run_ui(input_at_scale(screen, 1.0), |ui| options_bar(&mut app, ui));
    for _ in 0..3 {
        out.textures_delta.clear();
        out = ctx.run_ui(input_at_scale(screen, 1.0), |ui| options_bar(&mut app, ui));
    }
    out.textures_delta.clear();
    let texts: Vec<&str> = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.job.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(texts.contains(&"1.414"), "{texts:?}");
    assert_eq!(app.ui.tool_options.crop_ratio, "1.414:1", "showing the ratio leaves it alone");
}

#[test]
fn a_typed_crop_ratio_fits_from_the_frame_the_edit_started_with() {
    // #7752: typing 16 over 4 gave 1:3, then 16:3 inside that 100 x 300 frame (100 x 18.75).
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let Some(kind) = crate::theme::ThemeKind::ALL.into_iter().find(|k| {
        crate::theme::apply(&ctx, *k);
        Tokens::get(&ctx).pro
    }) else {
        return;
    };
    crate::theme::apply(&ctx, kind);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.tool = Tool::Crop;
    app.ui.tool_options.crop_shield.classic_mode = true;
    app.ui.tool_options.crop_ratio = "4:3".into();
    app.ui.crop_rect = Some([0.0, 0.0, 400.0, 300.0]);
    let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 120.0));
    let step = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
        let mut input = input_at_scale(screen, 1.0);
        input.events = events;
        ctx.run_ui(input, |ui| options_bar(app, ui)).textures_delta.clear();
    };
    for _ in 0..3 {
        step(&mut app, vec![]);
    }
    // The ratio's W field is the bar's first number field; focusing it selects its text.
    widgets::focus_first_field(&ctx, true);
    step(&mut app, vec![]);
    step(&mut app, vec![egui::Event::Text("1".into())]);
    assert_eq!(app.ui.tool_options.crop_ratio, "1:3");
    step(&mut app, vec![egui::Event::Text("6".into())]);
    step(&mut app, vec![]);
    assert_eq!(app.ui.tool_options.crop_ratio, "16:3");
    let r = app.ui.crop_rect.unwrap();
    let want = [0.0, 112.5, 400.0, 187.5];
    assert!(r.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9), "16:3 fitted once in 400 x 300: {r:?}");
}

#[test]
fn the_narrow_options_bar_preference_shortens_the_bar() {
    assert_eq!(options_bar_height(true, false), 36.0);
    assert_eq!(options_bar_height(true, true), 30.0);
    assert_eq!(options_bar_height(false, false), 42.0);
    assert_eq!(options_bar_height(false, true), 34.0);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 900.0));
    let run = |app: &mut PhotocraftApp| {
        let mut top = 0.0;
        let mut out = ctx.run_ui(input_at_scale(screen, 1.0), |ui| {
            options_bar(app, ui);
            top = ui.available_rect_before_wrap().top();
        });
        out.textures_delta.clear();
        top
    };
    let normal = run(&mut app);
    assert_eq!(normal, options_bar_height(Tokens::get(&ctx).pro, false));
    app.session.execute("prefs.set", json!({"path": "workspace.enableNarrowOptionsBar", "value": true})).unwrap();
    let narrow = run(&mut app);
    assert_eq!(narrow, options_bar_height(Tokens::get(&ctx).pro, true));
    assert!(narrow < normal, "narrow {narrow} must trim the bar under {normal}");
}

#[test]
fn narrow_tool_options_can_scroll_to_the_last_control() {
    for kind in crate::theme::ThemeKind::ALL {
        for width in [360.0, 800.0, 1280.0] {
            for scale in [1.0, 1.5, 2.0] {
                let ctx = egui::Context::default();
                egui_extras::install_image_loaders(&ctx);
                crate::theme::install_fonts(&ctx);
                crate::theme::apply(&ctx, kind);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
                let (tool, last_label) = if Tokens::get(&ctx).pro { (Tool::Pencil, "Auto Erase") } else { (Tool::Brush, "Pressure for Opacity") };
                app.ui.tool = tool;
                let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(width, 120.0));
                let mut output = ctx.run_ui(input_at_scale(screen, scale), |ui| options_bar(&mut app, ui));
                output.textures_delta.clear();
                for frame in 1..=6 {
                    output = ctx.run_ui(
                        egui::RawInput {
                            time: Some(f64::from(frame) * 0.1),
                            events: vec![
                                // Studio: above the controls, clear of the Brush's Mode dropdown, which takes the wheel
                                // to step through blend modes (#4512).
                                egui::Event::PointerMoved(if Tokens::get(&ctx).pro { pos2(width * 0.5, 20.0) } else { pos2(width * 0.5, 2.0) }),
                                egui::Event::MouseWheel {
                                    source: egui::MouseWheelSource::Unknown,
                                    unit: egui::MouseWheelUnit::Point,
                                    delta: vec2(-1500.0, 0.0),
                                    modifiers: egui::Modifiers::NONE,
                                    phase: egui::TouchPhase::Move,
                                },
                            ],
                            ..input_at_scale(screen, scale)
                        },
                        |ui| options_bar(&mut app, ui),
                    );
                    output.textures_delta.clear();
                }
                let (clip, label) = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text.contains(last_label) => Some((shape.clip_rect, text)),
                        _ => None,
                    })
                    .expect("scrolling reaches the final tool control");
                assert!(
                    clip.contains_rect(label.visual_bounding_rect()),
                    "{kind:?} at {width}: final control is clipped: {:?} / {clip:?}",
                    label.visual_bounding_rect()
                );
                assert!(label.visual_bounding_rect().right() <= width, "the final option stays inside the window");
            }
        }
    }
}
