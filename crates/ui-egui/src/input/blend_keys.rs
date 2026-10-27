//! Blend mode keys, as in Photoshop. ⇧+ and ⇧− step through the modes (#3102), and ⇧⌥ plus a
//! letter picks one (⇧⌥M Multiply, ⇧⌥S Screen…). With a painting tool that has a Mode of its own
//! (`tool_modes`: Brush, Pencil, Paint Bucket, Gradient, the stamps, History Brush, Healing) they
//! set that tool's mode within its menu and never the layer's. With a tool that has no blend mode
//! (Move, the selection tools, the Erasers…) they set every selected layer's mode.
//! The step list wraps at either end. Each press is one call of an engine command
//! (`tools.setBrush`, `layer.setProps` with every target layer), so it is one undo step.

use egui::{Event, Key};
use photocraft_color::BlendMode;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The mode `step` places after (or before, when negative) `current` in `modes`, wrapping around.
/// A mode that isn't listed steps from the top of the list.
pub fn stepped<T: Copy + PartialEq>(modes: &[T], current: T, step: i32) -> Option<T> {
    let n = i32::try_from(modes.len()).ok().filter(|n| *n > 0)?;
    let at = modes.iter().position(|m| *m == current).and_then(|i| i32::try_from(i).ok());
    let next = match at {
        Some(i) => (i + step).rem_euclid(n),
        None if step > 0 => 0,
        None => n - 1,
    };
    modes.get(usize::try_from(next).ok()?).copied()
}

/// What a blend mode key asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    Mode(BlendMode),
    /// Painting tools only: paint transparency.
    Clear,
    /// Painting tools only: paint behind the layer's pixels. Not implemented yet, so ⇧⌥Q is taken
    /// (it must not reach another binding) and does nothing.
    Behind,
}

/// Photoshop's ⇧⌥ letter for each mode. Darker Color, Lighter Color, Subtract and Divide have none.
pub fn letter_pick(key: Key) -> Option<Pick> {
    use BlendMode as B;
    Some(Pick::Mode(match key {
        Key::N => B::Normal,
        Key::I => B::Dissolve,
        Key::Q => return Some(Pick::Behind),
        Key::R => return Some(Pick::Clear),
        Key::P => B::PassThrough,
        Key::K => B::Darken,
        Key::M => B::Multiply,
        Key::B => B::ColorBurn,
        Key::A => B::LinearBurn,
        Key::G => B::Lighten,
        Key::S => B::Screen,
        Key::D => B::ColorDodge,
        Key::W => B::LinearDodge,
        Key::O => B::Overlay,
        Key::F => B::SoftLight,
        Key::H => B::HardLight,
        Key::V => B::VividLight,
        Key::J => B::LinearLight,
        Key::Z => B::PinLight,
        Key::L => B::HardMix,
        Key::E => B::Difference,
        Key::X => B::Exclusion,
        Key::U => B::Hue,
        Key::T => B::Saturation,
        Key::C => B::Color,
        Key::Y => B::Luminosity,
        _ => return None,
    }))
}

/// One key's request: a step through the list, or a mode picked by letter.
enum Press {
    Step(i32),
    Pick(Pick),
}

/// Consumes this frame's ⇧+ / ⇧− presses (⇧= on a US layout is `+`; on a German one
/// `layout_keys::apply` has already turned ⇧+ and ⇧- into `+` and `-`) and ⇧⌥ letters, and
/// applies them. Runs before the tool letters, which would otherwise take ⇧⌥M as M.
pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let presses: Vec<Press> = ctx.input_mut(|i| {
        let mut out = Vec::new();
        i.events.retain(|e| match e {
            Event::Key { key, physical_key, pressed: true, modifiers, .. } if modifiers.shift && !modifiers.command && !modifiers.ctrl => {
                if modifiers.alt {
                    // The layout's letter (⇧⌥Z on a German keyboard is Pin Light), else the
                    // physical one: on a Mac ⌥ turns the letter into another character (⇧⌥M
                    // types Â) that may arrive as some other key.
                    match letter_pick(*key).or_else(|| physical_key.and_then(letter_pick)) {
                        Some(p) => out.push(Press::Pick(p)),
                        None => return true,
                    }
                } else {
                    match key {
                        Key::Plus | Key::Equals => out.push(Press::Step(1)),
                        Key::Minus => out.push(Press::Step(-1)),
                        _ => return true,
                    }
                }
                false
            }
            _ => true,
        });
        out
    });
    for p in presses {
        match p {
            Press::Step(step) => press(app, step),
            Press::Pick(pick) => pick_mode(app, pick),
        }
    }
}

/// Photoshop's Healing Brush modes, less Replace (not implemented yet).
pub const HEALING_MODES: [BlendMode; 7] =
    [BlendMode::Normal, BlendMode::Multiply, BlendMode::Screen, BlendMode::Darken, BlendMode::Lighten, BlendMode::Color, BlendMode::Luminosity];

/// The mode a brush-based tool paints with: its brush's, or Normal for a Healing Brush whose brush
/// holds a mode its short menu lacks (the Brush's Overlay remembered from before it had a menu, a
/// preset with tool settings, `tools.setBrush`). The stroke, the dropdown and the keys share it.
pub fn brush_tool_mode(tool: Tool, mode: BlendMode) -> BlendMode {
    if tool == Tool::Healing && !HEALING_MODES.contains(&mode) { BlendMode::Normal } else { mode }
}

/// A painting tool's options-bar Mode menu in order, `None` being Clear (after Dissolve, on the
/// tools Photoshop offers it on). `None` for a tool without a blend mode control.
pub fn tool_modes(tool: Tool) -> Option<Vec<Option<BlendMode>>> {
    let layer = || BlendMode::LAYER_MODES.iter().copied().map(Some).collect::<Vec<_>>();
    Some(match tool {
        Tool::Brush | Tool::Pencil | Tool::PaintBucket => {
            let mut modes = layer();
            let at = modes.iter().position(|m| *m == Some(BlendMode::Dissolve)).map_or(modes.len(), |i| i + 1);
            modes.insert(at, None);
            modes
        }
        Tool::Gradient | Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush => layer(),
        Tool::Healing => HEALING_MODES.iter().copied().map(Some).collect(),
        _ => return None,
    })
}

/// Where the active tool's blend mode keys go.
enum Target {
    /// The tool's own Mode, one of these.
    Tool(Vec<Option<BlendMode>>),
    /// A painting tool with a blend mode in Photoshop but no Mode control here yet: the keys do
    /// nothing, and never reach the layer.
    Nothing,
    Layers,
}

fn target(tool: Tool) -> Target {
    if let Some(modes) = tool_modes(tool) {
        return Target::Tool(modes);
    }
    match tool {
        Tool::SpotHealing | Tool::ColorReplacement | Tool::Blur | Tool::Sharpen | Tool::Smudge | Tool::ArtHistoryBrush => Target::Nothing,
        // Photoshop's ⇧⌥S/M/H pick the Dodge and Burn Range and ⇧⌥D/S the Sponge's mode: never
        // the layer's.
        Tool::Dodge | Tool::Burn | Tool::Sponge => Target::Nothing,
        // No blend mode of their own (the Erasers, the Mixer Brush…): the layer's, as in Photoshop.
        _ => Target::Layers,
    }
}

/// The active tool's mode, `None` being Clear. A brush-based tool's lives in its own brush.
pub fn tool_mode(app: &mut PhotocraftApp) -> Option<BlendMode> {
    let o = &app.ui.tool_options;
    match app.ui.tool {
        Tool::Brush if o.brush_clear => None,
        Tool::Pencil if o.pencil_clear => None,
        Tool::PaintBucket if o.bucket_clear => None,
        Tool::PaintBucket => Some(o.bucket_blend_mode),
        Tool::Gradient => Some(o.gradient_blend_mode),
        _ => {
            crate::paint_mouse::sync_tool_brush(app);
            Some(brush_tool_mode(app.ui.tool, app.session.tools.brush.mode))
        }
    }
}

/// Sets the active tool's mode; `None` is Clear.
fn set_tool_mode(app: &mut PhotocraftApp, mode: Option<BlendMode>) {
    match app.ui.tool {
        Tool::Gradient => {
            if let Some(m) = mode {
                set_gradient_mode(app, m);
            }
            return;
        }
        Tool::PaintBucket => {
            app.ui.tool_options.bucket_clear = mode.is_none();
            if let Some(m) = mode {
                app.ui.tool_options.bucket_blend_mode = m;
            }
            return;
        }
        Tool::Brush => app.ui.tool_options.brush_clear = mode.is_none(),
        Tool::Pencil => app.ui.tool_options.pencil_clear = mode.is_none(),
        _ => {}
    }
    crate::paint_mouse::sync_tool_brush(app);
    if let Some(m) = mode
        && let Ok(m) = serde_json::to_value(m)
    {
        let _ = app.run("tools.setBrush", json!({ "brush": { "mode": m } }));
    }
}

fn set_gradient_mode(app: &mut PhotocraftApp, mode: BlendMode) {
    let before = app.ui.tool_options.clone();
    app.ui.tool_options.gradient_blend_mode = mode;
    // Live mode: a selected gradient fill layer follows, as with the dropdown.
    crate::gradient_ui::options_changed(app, &before);
}

/// Sets `mode` on every selected layer that can take it, in one undo step: the locked Background
/// has no blend mode, and only groups pass through.
fn set_layers_mode(app: &mut PhotocraftApp, mode: BlendMode) {
    let Some(st) = app.session.active() else { return };
    let ids: Vec<u64> = st
        .selected_layers()
        .into_iter()
        .filter_map(|id| st.doc.layer(id))
        .filter(|l| !crate::doc_props_ui::is_background(&st.doc, l) && (mode != BlendMode::PassThrough || l.is_group()) && l.blend != mode)
        .map(|l| l.id.0)
        .collect();
    if !ids.is_empty() {
        let _ = app.run("layer.setProps", json!({ "layers": ids, "blend": mode.label() }));
    }
}

/// One ⇧+ (`step` 1) or ⇧− (`step` -1) press. Selected layers all take the mode after the active
/// layer's.
pub fn press(app: &mut PhotocraftApp, step: i32) {
    match target(app.ui.tool) {
        Target::Tool(modes) => {
            let current = tool_mode(app);
            if let Some(mode) = stepped(&modes, current, step) {
                set_tool_mode(app, mode);
            }
        }
        Target::Nothing => {}
        Target::Layers => {
            let Some(st) = app.session.active() else { return };
            let Some(layer) = st.active_layer.and_then(|id| st.doc.layer(id)) else { return };
            // The locked Background has no blend mode.
            if crate::doc_props_ui::is_background(&st.doc, layer) {
                return;
            }
            let modes: Vec<BlendMode> = std::iter::once(BlendMode::PassThrough).filter(|_| layer.is_group()).chain(BlendMode::LAYER_MODES).collect();
            if let Some(mode) = stepped(&modes, layer.blend, step) {
                set_layers_mode(app, mode);
            }
        }
    }
}

/// One ⇧⌥ letter press. A mode the target's menu doesn't have (Clear or Behind for layers, Pass
/// Through for a painting tool, Overlay for the Healing Brush) does nothing.
pub fn pick_mode(app: &mut PhotocraftApp, pick: Pick) {
    let want = match pick {
        Pick::Mode(m) => Some(m),
        Pick::Clear => None,
        Pick::Behind => return,
    };
    match target(app.ui.tool) {
        Target::Tool(modes) if modes.contains(&want) => set_tool_mode(app, want),
        Target::Layers => {
            if let Some(m) = want {
                set_layers_mode(app, m);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
