//! The canvas Shift+right-click menu of the painting tools, as in the reference application: Edit
//! Brush… (the Brush Preset picker a plain right-click opens) and the tool's own Mode list from
//! its options bar (blend modes, the Eraser's Brush / Pencil / Block, the Dodge and Burn Range,
//! the Sponge's mode). Every row sets exactly what the tool's options-bar control sets, through
//! the same command (`tools.setBrush` for a brush-based tool's mode) or tool option.

use egui::{Context, Id};
use photocraft_doc::BlendMode;
use serde_json::{Value, json};

use crate::{PhotocraftApp, state::Tool, theme::Tokens};

/// One row of the menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Item {
    /// Opens the Brush Preset picker at the click point.
    EditBrush,
    Separator,
    /// The tool's blend mode.
    Blend(BlendMode),
    /// Painting-only: paint behind the layer's pixels (Brush and Pencil).
    Behind,
    /// Painting-only: paint transparency (Brush, Pencil, Paint Bucket).
    Clear,
    /// The Eraser's Mode: `brush`, `pencil` or `block`.
    Eraser(&'static str),
    /// The Dodge and Burn Range: `shadows`, `midtones` or `highlights`.
    Range(&'static str),
    /// The Sponge's mode: `desaturate` or `saturate`.
    Sponge(&'static str),
}

impl Item {
    /// The English label (translated when drawn); `None` for a separator.
    pub(crate) fn label(self) -> Option<&'static str> {
        Some(match self {
            Item::EditBrush => "Edit Brush…",
            Item::Separator => return None,
            Item::Blend(m) => m.label(),
            Item::Behind => "Behind",
            Item::Clear => "Clear",
            Item::Eraser(m) => match m {
                "pencil" => "Pencil",
                "block" => "Block",
                _ => "Brush",
            },
            Item::Range(r) => match r {
                "shadows" => "Shadows",
                "highlights" => "Highlights",
                _ => "Midtones",
            },
            Item::Sponge(m) => match m {
                "saturate" => "Saturate",
                _ => "Desaturate",
            },
        })
    }
}

/// The Brush's and Pencil's Mode list in the options bar's order, Behind and Clear after Dissolve:
/// `(mode, behind)`, `(None, false)` being Clear.
pub(crate) fn choices() -> Vec<((Option<BlendMode>, bool), &'static str)> {
    let mut choices: Vec<_> = BlendMode::LAYER_MODES.iter().map(|m| ((Some(*m), false), m.label())).collect();
    let at = choices.iter().position(|o| o.0 == (Some(BlendMode::Dissolve), false)).map_or(choices.len(), |i| i + 1);
    choices.insert(at, ((None, true), "Behind"));
    choices.insert(at + 1, ((None, false), "Clear"));
    choices
}

/// The blend modes in their menu groups (a separator before Darken, Lighten, Overlay, Difference
/// and Hue), with Behind and/or Clear after Dissolve.
fn blend_items(behind: bool, clear: bool) -> Vec<Item> {
    let mut out = Vec::new();
    for m in BlendMode::LAYER_MODES {
        if matches!(m, BlendMode::Darken | BlendMode::Lighten | BlendMode::Overlay | BlendMode::Difference | BlendMode::Hue) {
            out.push(Item::Separator);
        }
        out.push(Item::Blend(m));
        if m == BlendMode::Dissolve {
            if behind {
                out.push(Item::Behind);
            }
            if clear {
                out.push(Item::Clear);
            }
        }
    }
    out
}

/// `tool`'s menu, top to bottom. Empty for a tool without one (its Shift+right-click keeps its
/// other route). Only what the tool's options bar offers is listed: the reference application's
/// Behind for the stamps, History Brush, Gradient and Paint Bucket, the Blur, Sharpen and Smudge
/// modes, the Spot Healing modes and the Healing Brush's source and Replace have no setting here.
pub(crate) fn items(tool: Tool) -> Vec<Item> {
    let with_brush = |rest: Vec<Item>| std::iter::once(Item::EditBrush).chain(std::iter::once(Item::Separator)).chain(rest).collect::<Vec<_>>();
    match tool {
        Tool::Brush | Tool::Pencil => with_brush(blend_items(true, true)),
        Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush => with_brush(blend_items(false, false)),
        Tool::MixerBrush | Tool::ColorReplacement | Tool::BackgroundEraser | Tool::Blur | Tool::Sharpen | Tool::Smudge => vec![Item::EditBrush],
        Tool::Eraser => ["brush", "pencil", "block"].into_iter().map(Item::Eraser).collect(),
        Tool::Dodge | Tool::Burn => with_brush(["shadows", "midtones", "highlights"].into_iter().map(Item::Range).collect()),
        Tool::Sponge => with_brush(["desaturate", "saturate"].into_iter().map(Item::Sponge).collect()),
        Tool::Healing => crate::blend_keys::HEALING_MODES.into_iter().map(Item::Blend).collect(),
        Tool::Gradient => blend_items(false, false),
        Tool::PaintBucket => blend_items(false, true),
        _ => Vec::new(),
    }
}

/// The rows' labels, separators left out (what `ui.inspect` reports as `choices`).
pub(crate) fn labels(tool: Tool) -> Vec<&'static str> {
    items(tool).into_iter().filter_map(Item::label).collect()
}

/// Is a row available? Behind and Clear need transparency: greyed on the Background layer.
pub(crate) fn enabled(app: &PhotocraftApp, item: Item) -> bool {
    match item {
        Item::Separator => false,
        Item::Behind | Item::Clear => !app
            .session
            .active()
            .is_some_and(|st| st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| crate::doc_props_ui::is_background(&st.doc, l))),
        _ => true,
    }
}

/// The checked row: `tool`'s current mode, as its options bar shows it.
pub(crate) fn current(app: &PhotocraftApp, tool: Tool) -> Option<Item> {
    let o = &app.ui.tool_options;
    let brush = &app.session.tools.brush;
    let painting = |clear: bool| {
        if clear {
            Item::Clear
        } else if brush.behind {
            Item::Behind
        } else {
            Item::Blend(brush.mode)
        }
    };
    let item = match tool {
        Tool::Brush => painting(o.brush_clear),
        Tool::Pencil => painting(o.pencil_clear),
        Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush | Tool::Healing => Item::Blend(crate::blend_keys::brush_tool_mode(tool, brush.mode)),
        Tool::Gradient => Item::Blend(o.gradient_blend_mode),
        Tool::PaintBucket if o.bucket_clear => Item::Clear,
        Tool::PaintBucket => Item::Blend(o.bucket_blend_mode),
        _ => return items(tool).into_iter().find(|i| ui_value(app, *i).is_some_and(|v| v)),
    };
    Some(item)
}

/// Whether a tool-option row (Eraser, Range, Sponge) is the current value; `None` for other rows.
fn ui_value(app: &PhotocraftApp, item: Item) -> Option<bool> {
    let o = &app.ui.tool_options;
    Some(match item {
        Item::Eraser(m) => o.eraser_mode == m,
        Item::Range(r) => o.tone_range == r,
        Item::Sponge(m) => o.sponge_mode == m,
        _ => return None,
    })
}

/// Does the active tool have a menu for this gesture (Shift, without Alt, Ctrl or Command)?
pub(crate) fn is_gesture(tool: Tool, mods: egui::Modifiers) -> bool {
    mods.shift && !mods.alt && !mods.ctrl && !mods.command && !mods.mac_cmd && !items(tool).is_empty()
}

/// The modifiers that decide a right-button gesture: the ones held when the button went down.
/// egui reports `secondary_clicked` on release, so letting go of Shift a moment before the button
/// must not turn Shift+right-click into a plain right-click (an erase dab, with the Erase
/// preference). The control path decides on the press too.
pub(crate) fn gesture_mods(ctx: &Context, mods: egui::Modifiers) -> egui::Modifiers {
    let key = menu_id().with("press-mods");
    let (pressed, down, released) = ctx.input(|i| {
        (i.pointer.button_pressed(egui::PointerButton::Secondary), i.pointer.secondary_down(), i.pointer.button_released(egui::PointerButton::Secondary))
    });
    if pressed {
        ctx.data_mut(|d| d.insert_temp(key, mods));
        return mods;
    }
    if !down && !released {
        ctx.data_mut(|d| d.remove::<egui::Modifiers>(key));
        return mods;
    }
    let held = ctx.data(|d| d.get_temp::<egui::Modifiers>(key));
    if released {
        ctx.data_mut(|d| d.remove::<egui::Modifiers>(key));
    }
    held.unwrap_or(mods)
}

pub(crate) fn menu_id() -> Id {
    Id::unique("canvas-brush-mode-menu")
}

/// Opens the active tool's menu at screen point `pos`.
pub(crate) fn open(app: &mut PhotocraftApp, ctx: &Context, pos: [f32; 2]) {
    if !pos.iter().all(|v| v.is_finite()) || items(app.ui.tool).is_empty() {
        return;
    }
    // The checked row reads the tool's own brush.
    crate::paint_mouse::sync_tool_brush(app);
    app.ui.brush_picker = None;
    app.ui.layer_menu = None;
    app.ui.canvas_tool_menu = None;
    app.ui.brush_mode_menu = Some(pos);
    app.ui.brush_mode_menu_tool = Some(app.ui.tool);
    crate::canvas_popup::mark_open(ctx, menu_id());
}

fn close(app: &mut PhotocraftApp) {
    app.ui.brush_mode_menu = None;
    app.ui.brush_mode_menu_tool = None;
}

/// Is the open menu still the active tool's? A tool change closes it.
fn current_tool(app: &PhotocraftApp) -> bool {
    app.ui.brush_mode_menu_tool == Some(app.ui.tool)
}

/// A press outside dismisses the popup, and consumes that entire canvas gesture through Up.
pub(crate) fn blocks_canvas(app: &PhotocraftApp, ctx: &Context) -> bool {
    let key = menu_id().with("dismiss-pointer");
    if ctx.data(|d| d.get_temp::<bool>(key)).unwrap_or(false) {
        if !ctx.input(|i| i.pointer.any_down()) {
            ctx.data_mut(|d| d.remove::<bool>(key));
        }
        return true;
    }
    if app.ui.brush_mode_menu.is_some() {
        if let Some(rect) = ctx.memory(|m| m.area_rect(menu_id()))
            && crate::canvas_popup::outside_pressed(ctx, menu_id(), rect)
        {
            ctx.data_mut(|d| d.insert_temp(key, true));
        }
        return true;
    }
    false
}

/// Document-coordinate control pointers dismiss; menu rows are chosen with screen-space ui.click
/// or `ui.context.choose`.
pub(crate) fn blocks_control(app: &mut PhotocraftApp, ctx: &Context, ev: crate::canvas::ToolEvent) -> bool {
    use crate::canvas::ToolEvent;
    let key = menu_id().with("control-dismiss");
    let blocked = ctx.data(|d| d.get_temp::<bool>(key)).unwrap_or(false);
    if blocked {
        if matches!(ev, ToolEvent::Up { .. }) {
            ctx.data_mut(|d| d.remove::<bool>(key));
        }
        return true;
    }
    if app.ui.brush_mode_menu.is_some() {
        if !current_tool(app) {
            close(app);
            return false;
        }
        if matches!(ev, ToolEvent::Down { .. }) {
            close(app);
            ctx.data_mut(|d| d.insert_temp(key, true));
        }
        return true;
    }
    false
}

/// The engine command a row runs, if any: a brush-based tool's mode goes through
/// `tools.setBrush`, as its options-bar dropdown does. The Brush and Pencil also set or clear
/// Behind (Clear keeps the underlying mode, as the options bar does).
pub(crate) fn engine_call(app: &PhotocraftApp, tool: Tool, item: Item) -> Option<(&'static str, Value)> {
    let brush = match (tool, item) {
        (Tool::Brush | Tool::Pencil, Item::Blend(m)) => json!({"mode": m, "behind": false}),
        (Tool::Brush | Tool::Pencil, Item::Behind) => json!({"mode": BlendMode::Normal, "behind": true}),
        (Tool::Brush | Tool::Pencil, Item::Clear) => json!({"mode": app.session.tools.brush.mode, "behind": false}),
        (Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush | Tool::Healing, Item::Blend(m)) => json!({"mode": m}),
        _ => return None,
    };
    Some(("tools.setBrush", json!({ "brush": brush })))
}

/// The row of the open menu whose English label is `label`, if it can be chosen now.
pub(crate) fn find(app: &PhotocraftApp, label: &str) -> Result<Item, String> {
    if app.ui.brush_mode_menu.is_none() || !current_tool(app) {
        return Err("no Shift+right-click menu is open".into());
    }
    let item = items(app.ui.tool).into_iter().find(|i| i.label() == Some(label)).ok_or_else(|| format!("the menu has no row `{label}`"))?;
    if !enabled(app, item) {
        return Err(format!("`{label}` is unavailable"));
    }
    Ok(item)
}

/// Chooses `item` for the active tool and closes the menu.
pub(crate) fn choose(app: &mut PhotocraftApp, ctx: &Context, item: Item) {
    let tool = app.ui.tool;
    let pos = app.ui.brush_mode_menu;
    close(app);
    if !enabled(app, item) {
        return;
    }
    crate::paint_mouse::sync_tool_brush(app);
    if let Some((id, params)) = engine_call(app, tool, item) {
        match app.run(id, params) {
            Ok(_) => match tool {
                Tool::Brush => app.ui.tool_options.brush_clear = item == Item::Clear,
                Tool::Pencil => app.ui.tool_options.pencil_clear = item == Item::Clear,
                _ => {}
            },
            Err(e) => app.ui.status = e,
        }
        return;
    }
    match item {
        Item::EditBrush => {
            if let Some(at) = pos {
                crate::brush_picker::open(&mut app.ui, ctx, at);
            }
        }
        Item::Eraser(m) => app.ui.tool_options.eraser_mode = m.into(),
        Item::Range(r) => app.ui.tool_options.tone_range = r.into(),
        Item::Sponge(m) => app.ui.tool_options.sponge_mode = m.into(),
        Item::Blend(m) if tool == Tool::Gradient => crate::blend_keys::set_gradient_mode(app, m),
        Item::Blend(m) if tool == Tool::PaintBucket => {
            app.ui.tool_options.bucket_clear = false;
            app.ui.tool_options.bucket_blend_mode = m;
        }
        Item::Clear if tool == Tool::PaintBucket => app.ui.tool_options.bucket_clear = true,
        _ => {}
    }
}

pub(crate) fn show(app: &mut PhotocraftApp, ctx: &Context) {
    let Some([x, y]) = app.ui.brush_mode_menu else { return };
    if !current_tool(app) || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        let same_tool = current_tool(app);
        close(app);
        // Escape also consumes the release of a right-button gesture still in flight.
        if same_tool && ctx.input(|i| i.pointer.secondary_down()) {
            ctx.data_mut(|d| d.insert_temp(menu_id().with("dismiss-pointer"), true));
        }
        return;
    }
    crate::paint_mouse::sync_tool_brush(app);
    let tool = app.ui.tool;
    let rows: Vec<(Item, bool)> = items(tool).into_iter().map(|i| (i, enabled(app, i))).collect();
    let current = current(app, tool);
    let screen = crate::work_area::visible_rect(ctx);
    let id = menu_id();
    let margin = ctx.global_style().spacing.menu_margin.sum() + egui::Vec2::splat(2.0);
    let content: f32 = rows.iter().map(|(i, _)| if *i == Item::Separator { 7.0 } else { 22.0 }).sum();
    let height = content.min((screen.height() - margin.y).max(0.0));
    let width = (screen.width() - margin.x).clamp(0.0, 190.0);
    let size = ctx.memory(|m| m.area_rect(id)).map_or(egui::vec2(width + margin.x, height + margin.y), |r| r.size());
    let pos = egui::pos2(x.min(screen.right() - size.x).max(screen.left()), y.min(screen.bottom() - size.y).max(screen.top()));
    let mut picked = None;
    let area = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).constrain_to(screen).show(ctx, |ui| {
        let t = Tokens::get(ctx);
        egui::Frame::menu(ui.style()).fill(t.card).stroke(egui::Stroke::new(1.0, t.card_border)).show(ui, |ui| {
            ui.set_width(width);
            ui.spacing_mut().item_spacing.y = 0.0;
            crate::widgets::menu_scroll(ui, |ui| {
                for (item, on) in rows {
                    let Some(label) = item.label() else {
                        ui.separator();
                        continue;
                    };
                    let selected = Some(item) == current;
                    let color = if selected && t.bevel { t.accent_text } else { t.text };
                    let text = if on { egui::RichText::new(tl!(label)).color(color) } else { egui::RichText::new(tl!(label)) };
                    let row = egui::Button::selectable(selected, text).min_size(egui::vec2(width, 22.0));
                    if ui.add_enabled(on, row).clicked() {
                        picked = Some(item);
                    }
                }
            });
        });
    });
    if let Some(item) = picked {
        choose(app, ctx, item);
    } else if crate::canvas_popup::outside_pressed(ctx, id, area.response.rect) {
        close(app);
    }
}
