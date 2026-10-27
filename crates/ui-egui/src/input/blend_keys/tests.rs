use super::*;

fn app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.tool = tool;
    app
}

fn layer_blend(app: &PhotocraftApp) -> BlendMode {
    let st = app.session.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().blend
}

/// Feeds one key press with `modifiers` through the app's shortcut handling.
fn key(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, modifiers: egui::Modifiers) {
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

/// #3102: on a German layout ⇧+ types `*`, which egui reports as the physical `]`, and ⇧- types
/// `_` as the physical `/`. With the Brush they step its mode, not its hardness; a US ⇧] (`}`)
/// still raises the hardness.
#[test]
fn german_shift_plus_and_minus_step_the_mode_and_us_shift_bracket_keeps_the_hardness() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    a.session.tools.brush.hardness = 0.5;
    let shift = egui::Modifiers::SHIFT;
    let typed = |a: &mut PhotocraftApp, key: Key, physical_key: Key, text: &str| {
        let press = egui::Event::Key { key, physical_key: Some(physical_key), pressed: true, repeat: false, modifiers: shift };
        let raw = egui::RawInput { events: vec![egui::Event::ModifiersChanged(shift), press, egui::Event::Text(text.into())], ..Default::default() };
        ctx.begin_pass(raw);
        crate::shortcuts::handle(a, &ctx);
        ctx.end_pass().textures_delta.clear();
    };
    typed(&mut a, Key::CloseBracket, Key::CloseBracket, "*");
    assert_eq!(a.session.tools.brush.mode, BlendMode::Dissolve);
    assert_eq!(a.session.tools.brush.hardness, 0.5, "not Increase Brush Hardness");
    typed(&mut a, Key::Slash, Key::Slash, "_");
    assert_eq!(a.session.tools.brush.mode, BlendMode::Normal);
    // US: ⇧] arrives as `}` from the physical `]`.
    typed(&mut a, Key::CloseCurlyBracket, Key::CloseBracket, "}");
    assert!(a.session.tools.brush.hardness > 0.5);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Normal);
}

#[test]
fn stepping_wraps_at_both_ends() {
    let m = BlendMode::LAYER_MODES;
    assert_eq!(stepped(&m, BlendMode::Normal, 1), Some(BlendMode::Dissolve));
    assert_eq!(stepped(&m, BlendMode::Normal, -1), m.last().copied());
    assert_eq!(stepped(&m, m[m.len() - 1], 1), Some(BlendMode::Normal));
    assert_eq!(stepped(&[], BlendMode::Normal, 1), None);
}

/// #3102: with the Move tool ⇧+ / ⇧− step the active layer's mode, one undo step each; with
/// the Brush they step the brush's mode and leave the layer alone.
#[test]
fn shift_plus_and_minus_step_the_layer_or_the_brush_mode() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Move);
    let undo = a.session.active().unwrap().history.past_len();
    key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
    assert_eq!(layer_blend(&a), BlendMode::Dissolve);
    key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
    assert_eq!(layer_blend(&a), BlendMode::Darken);
    key(&mut a, &ctx, Key::Minus, egui::Modifiers::SHIFT);
    assert_eq!(layer_blend(&a), BlendMode::Dissolve);
    assert_eq!(a.session.active().unwrap().history.past_len(), undo + 3, "one undo step per press");
    // Unshifted or ⌘ presses are not blend-mode keys.
    key(&mut a, &ctx, Key::Minus, egui::Modifiers::NONE);
    key(&mut a, &ctx, Key::Plus, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!(layer_blend(&a), BlendMode::Dissolve);
    a.ui.tool = Tool::Brush;
    key(&mut a, &ctx, Key::Minus, egui::Modifiers::SHIFT);
    assert_eq!(a.session.tools.brush.mode, BlendMode::LAYER_MODES[BlendMode::LAYER_MODES.len() - 1]);
    assert_eq!(layer_blend(&a), BlendMode::Dissolve, "the layer is untouched");
    // The Background keeps Normal; no document is a no-op.
    a.ui.tool = Tool::Move;
    let bg = a.session.active().unwrap().doc.layers[0].id;
    a.run("layer.select", json!({"layer": bg.0})).unwrap();
    key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
    assert_eq!(a.session.active().unwrap().doc.layers[0].blend, BlendMode::Normal);
    let mut none = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    press(&mut none, 1);
}

/// Feeds one ⇧⌥ press of `key`, reported by the platform as `physical` too.
fn alt_shift(app: &mut PhotocraftApp, ctx: &egui::Context, key: Key, physical: Key) {
    let m = egui::Modifiers::SHIFT | egui::Modifiers::ALT;
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(m), egui::Event::Key { key, physical_key: Some(physical), pressed: true, repeat: false, modifiers: m }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, ctx);
    ctx.end_pass().textures_delta.clear();
}

fn blend_of(app: &PhotocraftApp, i: usize) -> BlendMode {
    app.session.active().unwrap().doc.layers[i].blend
}

/// Photoshop's letters: 26 keys, each a different mode, and the four modes without one.
#[test]
fn every_letter_picks_its_own_mode() {
    let keys: Vec<Key> = "NIQRPKMBAGSDWOFHVJZLEXUTCY".chars().filter_map(|c| Key::from_name(&c.to_string())).collect();
    assert_eq!(keys.len(), 26);
    let picks: Vec<Pick> = keys.iter().filter_map(|k| letter_pick(*k)).collect();
    assert_eq!(picks.len(), keys.len());
    for (i, p) in picks.iter().enumerate() {
        assert!(!picks[..i].contains(p), "{p:?} has two letters");
    }
    assert_eq!(letter_pick(Key::M), Some(Pick::Mode(BlendMode::Multiply)));
    assert_eq!(letter_pick(Key::W), Some(Pick::Mode(BlendMode::LinearDodge)));
    assert_eq!(letter_pick(Key::Y), Some(Pick::Mode(BlendMode::Luminosity)));
    for m in [BlendMode::DarkerColor, BlendMode::LighterColor, BlendMode::Subtract, BlendMode::Divide] {
        assert!(!picks.contains(&Pick::Mode(m)), "{m:?} has no letter in Photoshop");
    }
    for k in [Key::Num1, Key::Space] {
        assert_eq!(letter_pick(k), None);
    }
}

/// With the Move tool ⇧⌥M makes the active layer Multiply in one undo step, and doesn't switch
/// to the Marquee tool's M. Clear and Behind are painting modes: a layer ignores them.
#[test]
fn alt_shift_letter_sets_the_layer_mode_and_keeps_the_tool() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Move);
    let undo = a.session.active().unwrap().history.past_len();
    alt_shift(&mut a, &ctx, Key::M, Key::M);
    assert_eq!(layer_blend(&a), BlendMode::Multiply);
    assert_eq!(a.ui.tool, Tool::Move, "not the Marquee tool");
    assert_eq!(a.session.active().unwrap().history.past_len(), undo + 1);
    alt_shift(&mut a, &ctx, Key::R, Key::R);
    alt_shift(&mut a, &ctx, Key::Q, Key::Q);
    alt_shift(&mut a, &ctx, Key::P, Key::P);
    assert_eq!(layer_blend(&a), BlendMode::Multiply, "no Clear, Behind or Pass Through on a layer");
    assert_eq!(a.ui.tool, Tool::Move);
    // ⇧⌥ letters that are tool keys pick modes too.
    alt_shift(&mut a, &ctx, Key::S, Key::S);
    assert_eq!(layer_blend(&a), BlendMode::Screen);
    assert_eq!(a.ui.tool, Tool::Move);
}

/// A Mac's ⌥ turns the letter into another character; egui then reports the physical key.
/// A German layout's Z (physical Y) is still Pin Light: the typed letter wins.
#[test]
fn the_typed_letter_wins_and_the_physical_key_is_the_fallback() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Move);
    alt_shift(&mut a, &ctx, Key::Backtick, Key::O);
    assert_eq!(layer_blend(&a), BlendMode::Overlay);
    alt_shift(&mut a, &ctx, Key::Z, Key::Y);
    assert_eq!(layer_blend(&a), BlendMode::PinLight);
}

/// Every selected layer takes the mode, all in one undo step; the Background keeps Normal.
/// ⇧+ steps them all to the mode after the active layer's.
#[test]
fn mode_keys_set_every_selected_layer_in_one_undo_step() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Move);
    let ids: Vec<u64> = a.session.active().unwrap().doc.layers.iter().map(|l| l.id.0).collect();
    a.run("layer.new.layer", json!({})).unwrap();
    for id in &ids {
        a.run("layer.select", json!({"layer": id, "mode": "add"})).unwrap();
    }
    assert_eq!(a.session.active().unwrap().selected_layers().len(), 3);
    let undo = a.session.active().unwrap().history.past_len();
    alt_shift(&mut a, &ctx, Key::S, Key::S);
    assert_eq!([blend_of(&a, 0), blend_of(&a, 1), blend_of(&a, 2)], [BlendMode::Normal, BlendMode::Screen, BlendMode::Screen]);
    assert_eq!(a.session.active().unwrap().history.past_len(), undo + 1, "one undo step for both layers");
    key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
    assert_eq!([blend_of(&a, 1), blend_of(&a, 2)], [BlendMode::ColorDodge, BlendMode::ColorDodge]);
    assert_eq!(a.session.active().unwrap().history.past_len(), undo + 2);
    a.session.undo();
    assert_eq!([blend_of(&a, 1), blend_of(&a, 2)], [BlendMode::Screen, BlendMode::Screen]);
}

/// ⇧⌥P is Pass Through, for groups only.
#[test]
fn pass_through_is_for_groups() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Move);
    a.run("layer.new.group", json!({})).unwrap();
    alt_shift(&mut a, &ctx, Key::N, Key::N);
    assert_eq!(layer_blend(&a), BlendMode::Normal);
    alt_shift(&mut a, &ctx, Key::P, Key::P);
    assert_eq!(layer_blend(&a), BlendMode::PassThrough);
}

/// With the Brush the letters set the brush's mode, Clear included, and never the layer's;
/// ⇧+ from Dissolve reaches Clear, as in the dropdown. The Pencil keeps its own Clear.
#[test]
fn with_a_painting_tool_the_letters_set_only_its_mode() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    alt_shift(&mut a, &ctx, Key::M, Key::M);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Multiply);
    assert_eq!(layer_blend(&a), BlendMode::Normal, "the layer is untouched");
    assert_eq!(a.ui.tool, Tool::Brush);
    alt_shift(&mut a, &ctx, Key::R, Key::R);
    assert!(a.ui.tool_options.brush_clear);
    alt_shift(&mut a, &ctx, Key::P, Key::P);
    assert!(a.ui.tool_options.brush_clear, "no Pass Through for a brush");
    alt_shift(&mut a, &ctx, Key::I, Key::I);
    assert!(!a.ui.tool_options.brush_clear);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Dissolve);
    key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
    assert!(a.ui.tool_options.brush_clear, "Clear follows Dissolve");
    key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
    assert!(!a.ui.tool_options.brush_clear);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Darken);
    assert!(!a.ui.tool_options.pencil_clear);
    a.ui.tool = Tool::Pencil;
    alt_shift(&mut a, &ctx, Key::R, Key::R);
    assert!(a.ui.tool_options.pencil_clear);
    assert_eq!(layer_blend(&a), BlendMode::Normal);
}

/// The Gradient takes the letters; Smudge, which has a blend mode in Photoshop but no Mode
/// control here yet, ignores them rather than changing the layer.
#[test]
fn gradient_takes_the_letters_and_smudge_ignores_them() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Gradient);
    alt_shift(&mut a, &ctx, Key::O, Key::O);
    assert_eq!(a.ui.tool_options.gradient_blend_mode, BlendMode::Overlay);
    alt_shift(&mut a, &ctx, Key::R, Key::R);
    assert_eq!(a.ui.tool_options.gradient_blend_mode, BlendMode::Overlay, "no Clear for the Gradient");
    a.ui.tool = Tool::Smudge;
    alt_shift(&mut a, &ctx, Key::M, Key::M);
    assert_eq!(layer_blend(&a), BlendMode::Normal);
    assert_eq!(a.ui.tool, Tool::Smudge);
    let mut none = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    pick_mode(&mut none, Pick::Mode(BlendMode::Multiply));
}

/// With a Free Transform box open, ⇧⌥ letters and the number keys still go where the tool
/// sends them: the Brush's mode and opacity with the Brush, the layer's with the Move tool.
/// They never pick a tool, so the box stays open, and Enter keeps both the transform and the
/// layer's new mode. Puppet Warp keeps them to itself: its preview is a snapshot taken when it
/// started, so a change made meanwhile wouldn't show, and Esc wouldn't take it back.
#[test]
fn keys_follow_the_tool_while_a_transform_is_open_and_a_warp_keeps_them() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    a.run("select.rect", json!({"x": 4, "y": 4, "width": 12, "height": 12})).unwrap();
    a.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    a.run("select.deselect", json!({})).unwrap();
    a.sync_views();
    crate::transform_tool::begin(&mut a, &ctx).unwrap();
    crate::transform_tool::nudge(&mut a, 3.0, 0.0);
    let shift_alt = egui::Modifiers::SHIFT | egui::Modifiers::ALT;
    let opacity = |a: &PhotocraftApp| {
        let st = a.session.active().unwrap();
        st.doc.layer(st.active_layer.unwrap()).unwrap().opacity
    };
    // The Brush: its own mode and opacity.
    key(&mut a, &ctx, Key::M, shift_alt);
    assert_eq!(a.session.tools.brush.mode, BlendMode::Multiply);
    key(&mut a, &ctx, Key::Num5, egui::Modifiers::NONE);
    assert!((a.session.tools.brush.opacity - 0.5).abs() < 1e-6);
    assert_eq!((layer_blend(&a), opacity(&a)), (BlendMode::Normal, 1.0), "the layer is left alone");
    assert!(a.ui.transform.is_some(), "the box stays open");
    assert_eq!(a.ui.tool, Tool::Brush, "no tool letter");
    // The Move tool: the layer's.
    a.ui.tool = Tool::Move;
    a.opacity_keys = None;
    key(&mut a, &ctx, Key::E, shift_alt);
    key(&mut a, &ctx, Key::Num7, egui::Modifiers::NONE);
    assert_eq!(layer_blend(&a), BlendMode::Difference);
    assert!((opacity(&a) - 0.7).abs() < 1e-6, "{}", opacity(&a));
    assert!(a.ui.transform.is_some());
    key(&mut a, &ctx, Key::Enter, egui::Modifiers::NONE);
    assert!(a.ui.transform.is_none());
    let st = a.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(l.blend, BlendMode::Difference, "the commit keeps it");
    assert_eq!(l.surface().unwrap().content_bounds().x0, 7, "and the move");
    let l_opacity = l.opacity;
    // Puppet Warp: nothing reaches the layer, the Brush or the History.
    crate::distort_ui::menu(&mut a, &ctx, "edit.puppetWarp", &json!({})).unwrap().unwrap();
    assert!(a.distort.puppet.is_some());
    let undo = a.session.active().unwrap().history.past_len();
    a.opacity_keys = None;
    key(&mut a, &ctx, Key::S, shift_alt);
    key(&mut a, &ctx, Key::Plus, egui::Modifiers::SHIFT);
    key(&mut a, &ctx, Key::Num3, egui::Modifiers::NONE);
    a.ui.tool = Tool::Brush;
    key(&mut a, &ctx, Key::H, shift_alt);
    key(&mut a, &ctx, Key::Num9, egui::Modifiers::NONE);
    assert_eq!((layer_blend(&a), opacity(&a)), (BlendMode::Difference, l_opacity), "the layer under the warp is left alone");
    assert_eq!(a.session.tools.brush.mode, BlendMode::Multiply);
    assert!((a.session.tools.brush.opacity - 0.5).abs() < 1e-6);
    assert_eq!(a.session.active().unwrap().history.past_len(), undo, "no History step behind the warp");
    assert!(a.distort.puppet.is_some(), "the warp stays open");
    // Perspective Warp too.
    key(&mut a, &ctx, Key::Escape, egui::Modifiers::NONE);
    assert!(a.distort.puppet.is_none());
    a.ui.tool = Tool::Move;
    crate::distort_ui::menu(&mut a, &ctx, "edit.perspectiveWarp", &json!({})).unwrap().unwrap();
    assert!(a.distort.perspective.is_some());
    key(&mut a, &ctx, Key::S, shift_alt);
    key(&mut a, &ctx, Key::Num3, egui::Modifiers::NONE);
    assert_eq!((layer_blend(&a), opacity(&a)), (BlendMode::Difference, l_opacity));
    assert_eq!(a.session.active().unwrap().history.past_len(), undo);
    assert!(a.distort.perspective.is_some());
}

/// The stamps and the History Brush keep their mode in their own brush: the keys set it there,
/// never the layer's, and another tool's brush keeps its own.
#[test]
fn clone_stamp_pattern_stamp_and_history_brush_take_the_keys_into_their_own_brush() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Brush);
    alt_shift(&mut a, &ctx, Key::M, Key::M);
    for (tool, key, mode) in
        [(Tool::CloneStamp, Key::S, BlendMode::Screen), (Tool::PatternStamp, Key::O, BlendMode::Overlay), (Tool::HistoryBrush, Key::K, BlendMode::Darken)]
    {
        a.ui.tool = tool;
        assert_eq!(tool_mode(&mut a), Some(BlendMode::Normal), "{tool:?} starts in Normal");
        alt_shift(&mut a, &ctx, key, key);
        assert_eq!(tool_mode(&mut a), Some(mode), "{tool:?}");
        alt_shift(&mut a, &ctx, Key::R, Key::R);
        assert_eq!(tool_mode(&mut a), Some(mode), "{tool:?} has no Clear");
        key_press(&mut a, &ctx, 1);
        assert_ne!(tool_mode(&mut a), Some(mode), "{tool:?} steps");
        assert_eq!(layer_blend(&a), BlendMode::Normal, "{tool:?} leaves the layer alone");
    }
    a.ui.tool = Tool::Brush;
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Multiply), "the Brush keeps its own mode");
}

fn key_press(a: &mut PhotocraftApp, ctx: &egui::Context, step: i32) {
    key(a, ctx, if step > 0 { Key::Plus } else { Key::Minus }, egui::Modifiers::SHIFT);
}

/// The Healing Brush's menu is short: letters outside it do nothing, and ⇧+ steps within it.
#[test]
fn healing_brush_takes_only_its_own_modes() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Healing);
    alt_shift(&mut a, &ctx, Key::O, Key::O);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Normal), "Overlay isn't a Healing mode");
    alt_shift(&mut a, &ctx, Key::Y, Key::Y);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Luminosity));
    key_press(&mut a, &ctx, 1);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Normal), "wraps within the Healing list");
    key_press(&mut a, &ctx, 1);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Multiply));
    assert_eq!(layer_blend(&a), BlendMode::Normal);
}

/// A Healing Brush whose brush holds Overlay (outside its menu) counts as Normal for the keys,
/// as for its stroke: ⇧+ steps on to Multiply.
#[test]
fn healing_brush_with_a_mode_outside_its_menu_steps_from_normal() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::Healing);
    crate::paint_mouse::sync_tool_brush(&mut a);
    a.session.tools.brush.mode = BlendMode::Overlay;
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Normal));
    key_press(&mut a, &ctx, 1);
    assert_eq!(tool_mode(&mut a), Some(BlendMode::Multiply));
}

/// The Paint Bucket has its own Mode, Clear included.
#[test]
fn paint_bucket_takes_the_keys_with_clear() {
    let ctx = egui::Context::default();
    let mut a = app(Tool::PaintBucket);
    alt_shift(&mut a, &ctx, Key::M, Key::M);
    assert_eq!(a.ui.tool_options.bucket_blend_mode, BlendMode::Multiply);
    alt_shift(&mut a, &ctx, Key::R, Key::R);
    assert!(a.ui.tool_options.bucket_clear);
    key_press(&mut a, &ctx, 1);
    assert!(!a.ui.tool_options.bucket_clear);
    assert_eq!(a.ui.tool_options.bucket_blend_mode, BlendMode::Darken, "Clear is followed by Darken");
    assert_eq!(layer_blend(&a), BlendMode::Normal);
}

/// Tools without a blend mode of their own (the Erasers, the Mixer Brush) pass the keys to the
/// selected layers, as Photoshop does.
#[test]
fn tools_without_a_blend_mode_set_the_layer() {
    let ctx = egui::Context::default();
    for (tool, key, mode) in
        [(Tool::Eraser, Key::M, BlendMode::Multiply), (Tool::MagicEraser, Key::S, BlendMode::Screen), (Tool::MixerBrush, Key::E, BlendMode::Difference)]
    {
        let mut a = app(tool);
        alt_shift(&mut a, &ctx, key, key);
        assert_eq!(layer_blend(&a), mode, "{tool:?}");
        assert_eq!(a.ui.tool, tool);
    }
}

/// Dodge, Burn and Sponge use ⇧⌥S/M/H and ⇧⌥D/S for their own Range and mode in Photoshop, so
/// neither the letters nor ⇧+ change the layer's blend mode.
#[test]
fn dodge_burn_and_sponge_leave_the_layer_alone() {
    let ctx = egui::Context::default();
    for tool in [Tool::Dodge, Tool::Burn, Tool::Sponge] {
        let mut a = app(tool);
        for k in [Key::S, Key::M, Key::H, Key::D] {
            alt_shift(&mut a, &ctx, k, k);
        }
        key_press(&mut a, &ctx, 1);
        assert_eq!(layer_blend(&a), BlendMode::Normal, "{tool:?}");
        assert_eq!(a.ui.tool, tool);
    }
}

#[test]
fn art_history_brush_leaves_the_layer_alone() {
    // Photoshop's Art History Brush has a Mode of its own; ours has no Mode control yet.
    let ctx = egui::Context::default();
    let mut a = app(Tool::ArtHistoryBrush);
    alt_shift(&mut a, &ctx, Key::M, Key::M);
    key_press(&mut a, &ctx, 1);
    assert_eq!(layer_blend(&a), BlendMode::Normal);
    assert_eq!(a.ui.tool, Tool::ArtHistoryBrush);
}
