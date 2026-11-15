//! Number keys set opacity (#352): 1 = 10% … 9 = 90%, 0 = 100%; two digits
//! typed quickly set an exact value (4 then 5 = 45%; 0 then 0 = 100% for the Brush and the Pencil, #11812). With the Brush, Pencil or
//! Eraser they set the brush's Opacity (⇧: Flow, swapped while airbrush build-up is on; the Pencil has no Flow, so its ⇧
//! digits set Opacity too, #11967); with the Gradient or Paint Bucket their
//! Opacity; with tools that have no opacity of their own (Move, the selection tools, Type…) the
//! active layer's Opacity (⇧: Fill); with the Background Eraser its Tolerance (#11907). Tools whose
//! number is a strength or exposure are left alone.
//! ⌥+digit sets the Smoothing of the tools that show it (Brush, Pencil, Eraser; #11892).

use egui::{Event, Key};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// A second digit within this long of the first makes a two-digit value.
const TWO_DIGITS_MS: f64 = 800.0;

/// The first digit of a possible two-digit value: (digit, ⇧ held, when).
pub type Pending = Option<(u8, bool, f64)>;

/// What a tool's number keys change.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    /// The shared brush settings' opacity (⇧: flow, when the tool has one).
    Brush {
        flow: bool,
    },
    /// The Gradient / Paint Bucket opacity option.
    ToolOpacity,
    /// The Background Eraser's Tolerance option.
    BgTolerance,
    /// The active layer's opacity (⇧: fill).
    Layer,
    None,
}

fn target(tool: Tool) -> Target {
    match tool {
        Tool::Brush | Tool::Eraser | Tool::CloneStamp | Tool::HistoryBrush => Target::Brush { flow: true },
        Tool::Pencil => Target::Brush { flow: false },
        Tool::Gradient | Tool::PaintBucket => Target::ToolOpacity,
        Tool::BackgroundEraser => Target::BgTolerance,
        // Their options-bar number is a strength, exposure or tolerance, not this opacity.
        t if t.is_brushlike() || t == Tool::MagicEraser => Target::None,
        _ => Target::Layer,
    }
}

fn digit(k: Key) -> Option<u8> {
    Some(match k {
        Key::Num0 => 0,
        Key::Num1 => 1,
        Key::Num2 => 2,
        Key::Num3 => 3,
        Key::Num4 => 4,
        Key::Num5 => 5,
        Key::Num6 => 6,
        Key::Num7 => 7,
        Key::Num8 => 8,
        Key::Num9 => 9,
        _ => return None,
    })
}

/// The tools whose options bar has a Smoothing field, which ⌥+digit sets (#11892).
fn has_smoothing(tool: Tool) -> bool {
    matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser)
}

/// One digit press of this frame: a plain or ⇧ digit, or an ⌥ digit for Smoothing.
enum Press {
    Digit(u8, bool),
    Smoothing(u8),
}

/// Consumes this frame's unmodified (or ⇧) digit presses and applies them, and ⌥+digit presses
/// for the tools with a Smoothing field, in the order they were typed.
pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let smoothing = has_smoothing(app.ui.tool);
    let presses: Vec<Press> = ctx.input_mut(|i| {
        let mut out = Vec::new();
        i.events.retain(|e| match e {
            // ⌥+digit: one tens value each (1 = 10% … 0 = 100%), never two digits.
            Event::Key { key, physical_key, pressed: true, repeat, modifiers }
                if smoothing && modifiers.alt && !modifiers.shift && !modifiers.command && !modifiers.ctrl =>
            {
                match physical_key.and_then(digit).or_else(|| digit(*key)) {
                    Some(d) => {
                        if !repeat {
                            out.push(Press::Smoothing(d));
                        }
                        false
                    }
                    None => true,
                }
            }
            // The physical key, so ⇧+digit is still a digit whatever symbol the layout puts there.
            Event::Key { key, physical_key, pressed: true, repeat, modifiers } if !modifiers.command && !modifiers.ctrl && !modifiers.alt => {
                match physical_key.and_then(digit).or_else(|| digit(*key)) {
                    Some(d) => {
                        // A held key's auto-repeat is still one digit, not a second one typed
                        // quickly (holding 4 set 44%, #953); it is consumed all the same.
                        if !repeat {
                            out.push(Press::Digit(d, modifiers.shift));
                        }
                        false
                    }
                    None => true,
                }
            }
            _ => true,
        });
        out
    });
    let now = crate::gpu_canvas::now_ms();
    for p in presses {
        match p {
            Press::Digit(d, shift) => press(app, d, shift, now),
            Press::Smoothing(d) => {
                // An ⌥ digit ends a two-digit Opacity value: 4, ⌥3, 5 is 50%, not 45%.
                app.opacity_keys = None;
                let amount = if d == 0 { 1.0 } else { f64::from(d) / 10.0 };
                let _ = app.run("tools.setBrush", json!({ "brush": { "smoothing": { "enabled": true, "amount": amount } } }));
            }
        }
    }
}

/// One digit press at `now` (ms).
pub fn press(app: &mut PhotocraftApp, d: u8, shift: bool, now: f64) {
    let percent = match app.opacity_keys.take() {
        Some((first, s, at)) if s == shift && now - at < TWO_DIGITS_MS && now >= at => f32::from(first * 10 + d),
        _ => {
            app.opacity_keys = Some((d, shift, now));
            if d == 0 { 100.0 } else { f32::from(d) * 10.0 }
        }
    };
    apply(app, percent, shift);
}

fn apply(app: &mut PhotocraftApp, percent: f32, shift: bool) {
    let v = percent / 100.0;
    // With airbrush build-up on, the keys swap roles, as in Photoshop: plain digits set Flow, ⇧ digits Opacity.
    // A Pencil- or Block-mode Eraser has no build-up (its options bar disables Flow and the airbrush).
    let tool = app.ui.tool;
    let build_up = app.session.tools.brush.build_up && !crate::eraser_ui::pencil_mode(app, tool) && !crate::eraser_ui::block_mode(app, tool);
    let shift = shift ^ (build_up && matches!(target(tool), Target::Brush { flow: true }));
    match (target(app.ui.tool), shift) {
        // The Pencil has no Flow: its ⇧ digits set Opacity like the plain ones (#11967).
        (Target::Brush { flow: false }, _) | (Target::Brush { flow: true }, false) => {
            // The painting tools' Opacity fields (and `tools.setBrush`) start at 1%, so `00` sets the
            // 1% they actually paint with rather than a 0% they can't do. The Brush's and the
            // Pencil's `00` is 100%, as in Photoshop (#11812); only `00` gives 0 here.
            let v = match tool {
                Tool::Brush | Tool::Pencil if v <= 0.0 => 1.0,
                _ => v.max(0.01),
            };
            let _ = app.run("tools.setBrush", json!({ "brush": { "opacity": v } }));
        }
        // The options bar's Flow field starts at 1%.
        (Target::Brush { flow: true }, true) => {
            let _ = app.run("tools.setBrush", json!({ "brush": { "flow": v.max(0.01) } }));
        }
        (Target::ToolOpacity, false) => app.ui.tool_options.fill_opacity = percent,
        // The Tolerance field starts at 1%; `00` (0%) keeps today's value rather than guessing.
        (Target::BgTolerance, false) if percent >= 1.0 => app.ui.tool_options.bg_tolerance = percent,
        (Target::Layer, _) => {
            // The locked Background has no opacity or fill.
            let Some(st) = app.session.active() else { return };
            let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return };
            if crate::doc_props_ui::is_background(&st.doc, layer) {
                return;
            }
            let key = if shift { "fill" } else { "opacity" };
            let _ = app.run("layer.setProps", json!({ "layer": layer.id.0, key: v }));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(tool: Tool) -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.ui.tool = tool;
        app
    }

    fn layer(app: &PhotocraftApp) -> (f32, f32) {
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        (l.opacity, l.fill_opacity)
    }

    #[test]
    fn digits_set_the_brush_opacity_and_shift_digits_its_flow() {
        let mut a = app(Tool::Brush);
        press(&mut a, 5, false, 0.0);
        assert_eq!(a.session.tools.brush.opacity, 0.5);
        // Two quick digits: an exact value; a slow second digit starts over.
        press(&mut a, 4, false, 1000.0);
        press(&mut a, 5, false, 1300.0);
        assert!((a.session.tools.brush.opacity - 0.45).abs() < 1e-6);
        press(&mut a, 3, false, 5000.0);
        assert!((a.session.tools.brush.opacity - 0.3).abs() < 1e-6);
        press(&mut a, 0, false, 9000.0);
        assert_eq!(a.session.tools.brush.opacity, 1.0, "0 = 100%");
        press(&mut a, 0, false, 9100.0);
        assert_eq!(a.session.tools.brush.opacity, 1.0, "00 = 100%");
        press(&mut a, 2, true, 20000.0);
        assert!((a.session.tools.brush.flow - 0.2).abs() < 1e-6);
        press(&mut a, 0, true, 30000.0);
        assert_eq!(a.session.tools.brush.flow, 1.0);
        press(&mut a, 0, true, 30100.0);
        assert_eq!(a.session.tools.brush.flow, 0.01, "flow keeps its 1% floor");
        assert_eq!(layer(&a), (1.0, 1.0), "the layer is untouched");
        // The Pencil has no flow.
        let mut p = app(Tool::Pencil);
        press(&mut p, 7, true, 0.0);
        assert_eq!(p.session.tools.brush.flow, 1.0);
    }

    #[test]
    fn with_build_up_digits_set_flow_and_shift_digits_set_opacity() {
        let mut a = app(Tool::Brush);
        a.session.tools.brush.build_up = true;
        press(&mut a, 3, false, 0.0);
        assert!((a.session.tools.brush.flow - 0.3).abs() < 1e-6);
        assert_eq!(a.session.tools.brush.opacity, 1.0, "opacity is untouched");
        press(&mut a, 6, true, 5000.0);
        assert!((a.session.tools.brush.opacity - 0.6).abs() < 1e-6);
        assert!((a.session.tools.brush.flow - 0.3).abs() < 1e-6);
        // The Pencil has no flow, so build-up changes nothing for it.
        let mut p = app(Tool::Pencil);
        p.session.tools.brush.build_up = true;
        press(&mut p, 4, false, 0.0);
        assert!((p.session.tools.brush.opacity - 0.4).abs() < 1e-6);
        assert_eq!(p.session.tools.brush.flow, 1.0);
        // The Eraser swaps too, except in Pencil mode, which has no build-up.
        let mut e = app(Tool::Eraser);
        e.session.tools.brush.build_up = true;
        press(&mut e, 2, false, 0.0);
        assert!((e.session.tools.brush.flow - 0.2).abs() < 1e-6);
        e.ui.tool_options.eraser_mode = "pencil".into();
        press(&mut e, 7, false, 5000.0);
        assert!((e.session.tools.brush.opacity - 0.7).abs() < 1e-6);
        assert!((e.session.tools.brush.flow - 0.2).abs() < 1e-6);
    }

    #[test]
    fn history_brush_and_clone_stamp_digits_set_brush_opacity_and_flow() {
        for tool in [Tool::HistoryBrush, Tool::CloneStamp] {
            let mut a = app(tool);
            press(&mut a, 7, false, 0.0);
            assert!((a.session.tools.brush.opacity - 0.7).abs() < 1e-6);
            press(&mut a, 4, true, 1000.0);
            assert!((a.session.tools.brush.flow - 0.4).abs() < 1e-6);
            assert_eq!(layer(&a), (1.0, 1.0), "the layer is untouched");
        }
    }

    #[test]
    fn double_zero_gives_history_brush_and_clone_stamp_their_one_percent_minimum() {
        for tool in [Tool::HistoryBrush, Tool::CloneStamp] {
            let mut a = app(tool);
            press(&mut a, 0, false, 0.0);
            press(&mut a, 0, false, 100.0);
            assert!((a.session.tools.brush.opacity - 0.01).abs() < 1e-6, "{tool:?}");
        }
    }

    #[test]
    fn other_tools_set_the_layer_or_their_own_opacity() {
        let mut a = app(Tool::Move);
        let rev = a.session.active().unwrap().revision;
        press(&mut a, 6, false, 0.0);
        press(&mut a, 3, true, 5000.0);
        assert!((layer(&a).0 - 0.6).abs() < 1e-6 && (layer(&a).1 - 0.3).abs() < 1e-6);
        assert!(a.session.active().unwrap().revision > rev, "a layer change is an edit");
        a.ui.tool = Tool::PaintBucket;
        press(&mut a, 2, false, 9000.0);
        assert_eq!(a.ui.tool_options.fill_opacity, 20.0);
        // Dodge's number is its exposure: nothing changes here.
        a.ui.tool = Tool::Dodge;
        let before = (a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.fill_opacity);
        press(&mut a, 9, false, 20000.0);
        assert_eq!((a.session.tools.brush.clone(), layer(&a), a.ui.tool_options.fill_opacity), before);
        // The locked Background keeps its opacity; no document is a no-op.
        a.ui.tool = Tool::Move;
        let bg = a.session.active().unwrap().doc.layers[0].id;
        a.run("layer.select", json!({"layer": bg.0})).unwrap();
        press(&mut a, 5, false, 30000.0);
        assert_eq!(a.session.active().unwrap().doc.layers[0].opacity, 1.0);
        let mut none = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        press(&mut none, 5, false, 0.0);
    }

    #[test]
    fn number_key_events_reach_the_handler_and_modified_ones_do_not() {
        let mut a = app(Tool::Brush);
        let ctx = egui::Context::default();
        let key = |k: Key, m: egui::Modifiers| Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m };
        let input = egui::RawInput { events: vec![key(Key::Num7, egui::Modifiers::NONE), key(Key::Num3, egui::Modifiers::COMMAND)], ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            handle(&mut a, ui.ctx());
            assert_eq!(ui.input(|i| i.events.len()), 1, "⌘3 is left for its shortcut");
        });
        out.textures_delta.clear();
        assert!((a.session.tools.brush.opacity - 0.7).abs() < 1e-6);
    }

    #[test]
    fn a_held_digit_is_one_digit() {
        // The press and its first auto-repeat in one frame used to read as "4 then 4" = 44% (#953).
        let mut a = app(Tool::Brush);
        let ctx = egui::Context::default();
        let key = |repeat| Event::Key { key: Key::Num4, physical_key: Some(Key::Num4), pressed: true, repeat, modifiers: egui::Modifiers::NONE };
        let input = egui::RawInput { events: vec![key(false), key(true)], ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            handle(&mut a, ui.ctx());
            assert_eq!(ui.input(|i| i.events.len()), 0, "repeats are consumed, not passed on");
        });
        out.textures_delta.clear();
        assert!((a.session.tools.brush.opacity - 0.4).abs() < 1e-6, "{}", a.session.tools.brush.opacity);
        // A real second press of the same key still makes a two-digit value.
        press(&mut a, 4, false, crate::gpu_canvas::now_ms());
        assert!((a.session.tools.brush.opacity - 0.44).abs() < 1e-6, "{}", a.session.tools.brush.opacity);
    }

    /// Through the whole app: number keys and ⇧[ ⇧] from the canvas (no widget focused).
    #[test]
    fn keys_work_in_the_app() {
        use egui_kittest::Harness;
        let mut h = Harness::builder().with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut a = app(Tool::Brush);
            a.run("tools.setBrush", json!({"brush": {"hardness": 0.5, "opacity": 1.0, "flow": 1.0}})).unwrap();
            a
        });
        h.run_steps(4);
        let key = |h: &mut Harness<'_, PhotocraftApp>, k: Key, m: egui::Modifiers| {
            h.event(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
            h.event(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
            h.run_steps(2);
        };
        key(&mut h, Key::Num5, egui::Modifiers::NONE);
        key(&mut h, Key::CloseBracket, egui::Modifiers::SHIFT);
        let b = &h.state().session.tools.brush;
        assert_eq!((b.opacity, b.hardness), (0.5, 0.75));
        key(&mut h, Key::OpenBracket, egui::Modifiers::SHIFT);
        key(&mut h, Key::OpenBracket, egui::Modifiers::SHIFT);
        key(&mut h, Key::OpenBracket, egui::Modifiers::SHIFT);
        assert_eq!(h.state().session.tools.brush.hardness, 0.0, "stops at 0%");
        let size = h.state().session.tools.brush.size;
        key(&mut h, Key::CloseBracket, egui::Modifiers::NONE);
        assert!(h.state().session.tools.brush.size > size, "] still resizes");
    }
}
