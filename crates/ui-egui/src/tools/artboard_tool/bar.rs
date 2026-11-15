//! The Artboard tool's options bar controls (#11385): the size group (preset, W, H, Make Portrait,
//! Make Landscape) that edits the selected board, or the tool's own options when no board is
//! selected; the background swatch and dropdown for the boards the tool draws; the Align panel; and
//! the gear menu. The options live in [`ArtboardToolUi`](super::ArtboardToolUi).

use egui::Color32;
use photocraft_doc::{ArtboardBackground, Layer, LayerId};
use photocraft_engine::artboard_cmds::PRESETS;
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// The size preset a fresh bar (and Reset Tool) shows, with its width and height.
pub const DEFAULT_SIZE: (&str, u32, u32) = ("iPhone 8/7/6", 750, 1334);

/// The largest board side the bar accepts (the engine's limit).
const MAX_SIDE: f32 = 300_000.0;

/// The Align panel: three groups of entries. An entry is a label and the `layer.align.*` command
/// it runs; Distribute Widths / Heights have none yet (their behaviour is not measured), so they
/// show disabled.
pub const ALIGN_PANEL: [&[(&str, Option<&str>)]; 3] = [
    &[
        ("Left Edges", Some("layer.align.leftEdges")),
        ("Horizontal Centers", Some("layer.align.horizontalCenters")),
        ("Right Edges", Some("layer.align.rightEdges")),
    ],
    &[
        ("Top Edges", Some("layer.align.topEdges")),
        ("Vertical Centers", Some("layer.align.verticalCenters")),
        ("Bottom Edges", Some("layer.align.bottomEdges")),
    ],
    &[("Distribute Widths", None), ("Distribute Heights", None)],
];

/// The gear menu's entries, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gear {
    AutoNest,
    AutoSizeCanvas,
    KeepRelativePosition,
    ShrinkwrapOnSave,
}

/// Each gear entry and its label.
pub const GEAR: [(Gear, &str); 4] = [
    (Gear::AutoNest, "Auto-nest Layers"),
    (Gear::AutoSizeCanvas, "Auto-size Canvas"),
    (Gear::KeepRelativePosition, "Keep Relative Position During Layer Reordering"),
    (Gear::ShrinkwrapOnSave, "Shrinkwrap Canvas on Save"),
];

impl Gear {
    /// Is the entry checked?
    pub fn checked(self, o: &super::ArtboardToolUi) -> bool {
        match self {
            Gear::AutoNest => o.auto_nest,
            Gear::AutoSizeCanvas => o.auto_size_canvas,
            Gear::KeepRelativePosition => o.keep_relative_position,
            Gear::ShrinkwrapOnSave => o.shrinkwrap_on_save,
        }
    }

    /// Only Auto-nest Layers does something yet (the Move tool's auto-nesting, #10512); the others
    /// are drawn checked and disabled until their behaviour is measured.
    pub fn enabled(self) -> bool {
        self == Gear::AutoNest
    }
}

/// "The bar's size": the selected board's size, else the tool option's. `[width, height]` in pixels.
pub fn bar_size(app: &PhotocraftApp) -> [i64; 2] {
    match super::active_board(app) {
        Some((_, r)) => [r[2].saturating_sub(r[0]), r[3].saturating_sub(r[1])],
        None => [i64::from(app.ui.artboard_tool.width), i64::from(app.ui.artboard_tool.height)],
    }
}

/// `params` for `layer.new.artboard` plus the bar's background: `background`, and `color` for Other….
pub fn with_background(app: &PhotocraftApp, mut params: Value) -> Value {
    let o = &app.ui.artboard_tool;
    params["background"] = json!(o.background);
    if o.background == "custom" {
        params["color"] = json!(o.background_color);
    }
    params
}

/// Choose size preset `name` as the tool option: its size, no board involved. Unknown names and
/// Custom (empty) change nothing.
pub fn set_preset(app: &mut PhotocraftApp, name: &str) {
    if let Some((n, w, h)) = PRESETS.iter().find(|(n, _, _)| *n == name) {
        let o = &mut app.ui.artboard_tool;
        (o.preset, o.width, o.height) = (n.to_string(), *w, *h);
    }
}

/// Set the tool option size to `w` x `h` (clamped to 1..=300000); the preset becomes Custom, or
/// stays when the size is still the preset's.
pub fn set_size(app: &mut PhotocraftApp, w: f32, h: f32) {
    let clamp = |v: f32| if v.is_finite() { v.round().clamp(1.0, MAX_SIDE) as u32 } else { 1 };
    let (w, h) = (clamp(w), clamp(h));
    let o = &mut app.ui.artboard_tool;
    (o.width, o.height) = (w, h);
    if !PRESETS.iter().any(|(n, pw, ph)| *n == o.preset && (*pw, *ph) == (w, h)) {
        o.preset.clear();
    }
}

/// The size preset the bar shows for board `id`: the name stored on the board ("" = Custom).
pub fn board_preset(app: &PhotocraftApp, id: LayerId) -> String {
    app.session.active().and_then(|st| st.doc.layer(id)).and_then(Layer::artboard).map(|a| a.preset.clone()).unwrap_or_default()
}

/// The selected boards: each selected layer's artboard, once, in document order.
fn selected_boards(app: &PhotocraftApp) -> Vec<LayerId> {
    let Some(st) = app.session.active() else { return Vec::new() };
    let mut ids: Vec<LayerId> = Vec::new();
    for l in st.selected_layers() {
        if let Some(b) = st.doc.artboard_of(l)
            && !ids.contains(&b)
        {
            ids.push(b);
        }
    }
    ids
}

/// The background the bar shows as `(name, #rrggbb)`: the first selected board's, else the tool
/// option's. The colour is the custom one (or the option's remembered one for the named ones).
pub fn shown_background(app: &PhotocraftApp) -> (String, String) {
    let o = &app.ui.artboard_tool;
    let first = selected_boards(app).first().and_then(|id| app.session.active()?.doc.layer(*id)?.artboard().map(|a| a.background));
    match first {
        Some(ArtboardBackground::Custom(c)) => {
            let [r, g, b, _] = c.to_rgba8();
            ("custom".to_string(), format!("#{r:02x}{g:02x}{b:02x}"))
        }
        Some(b) => (b.name().to_string(), o.background_color.clone()),
        None => (o.background.clone(), o.background_color.clone()),
    }
}

/// Choose background `name` (`white`, `black`, `transparent` or `custom` with `color`, `#rrggbb`).
/// It is the tool option for the next drawn board, and with boards selected also the background of
/// every selected board in one `layer.artboard.set` step ("Edit Artboard" / "Edit Artboards").
pub fn set_background(app: &mut PhotocraftApp, name: &str, color: &str) -> Result<Value, String> {
    let custom = name == "custom";
    if custom {
        crate::color_picker_ui::parse_hex(color).ok_or("not a colour")?;
    }
    let mut boards = selected_boards(app);
    // A board whose custom colour already has this RGB keeps its own value: the swatch shows it
    // as 8-bit RGB, so OK in the picker without a change must not flatten its alpha or precision.
    if custom && let Some(want) = crate::color_picker_ui::parse_hex(color) {
        let hex = |c: [f32; 3]| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let st = app.session.active();
        boards.retain(|id| match st.and_then(|st| st.doc.layer(*id)?.artboard().map(|a| a.background)) {
            Some(ArtboardBackground::Custom(c)) => {
                let [r, g, b, _] = c.to_rgba8();
                [r, g, b] != hex(want)
            }
            _ => true,
        });
    }
    let mut out = json!({});
    if !boards.is_empty() {
        let mut params = json!({"layers": boards.iter().map(|b| b.0).collect::<Vec<_>>(), "background": name});
        if custom {
            params["color"] = json!(color);
        }
        out = app.run("layer.artboard.set", params)?;
    }
    let o = &mut app.ui.artboard_tool;
    o.background = name.to_string();
    if custom {
        o.background_color = color.to_string();
    }
    Ok(out)
}

/// OK in the Color Picker opened by Other…: the custom background is `color` (`#rrggbb`), for the
/// selected boards (if any) and the next drawn one.
pub fn set_custom_background(app: &mut PhotocraftApp, color: &str) -> Result<Value, String> {
    set_background(app, "custom", color)?;
    Ok(json!({"color": color}))
}

/// [`set_custom_background`] for OK in the picker. `unchanged` (the result is the colour the picker
/// opened with, after any Only Web Colors snap) leaves every selected board alone, with no command
/// and no history step: only the tool option for the next board follows. Otherwise boards that
/// already have the picked RGB are skipped one by one ([`set_background`]).
pub fn set_custom_background_from_picker(app: &mut PhotocraftApp, color: &str, unchanged: bool) -> Result<Value, String> {
    if !unchanged {
        return set_custom_background(app, color);
    }
    crate::color_picker_ui::parse_hex(color).ok_or("not a colour")?;
    let o = &mut app.ui.artboard_tool;
    o.background = "custom".to_string();
    o.background_color = color.to_string();
    Ok(json!({"color": color}))
}

/// Run Align panel entry `index` of group `group` on the selected boards. None for an entry that
/// has no command yet.
pub fn run_align(app: &mut PhotocraftApp, group: usize, index: usize) -> Option<Result<Value, String>> {
    let cmd = ALIGN_PANEL.get(group)?.get(index)?.1?;
    Some(app.run(cmd, json!({})))
}

/// The swatch colour of a background (`name`, with `hex` for custom).
fn background_color(name: &str, hex: &str) -> Color32 {
    match name {
        "black" => Color32::BLACK,
        "transparent" => Color32::TRANSPARENT,
        "custom" => crate::color_picker_ui::parse_hex(hex).map_or(Color32::WHITE, rgb32),
        _ => Color32::WHITE,
    }
}

fn rgb32(c: [f32; 3]) -> Color32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(b(c[0]), b(c[1]), b(c[2]))
}

/// Opens the Color Picker for the custom background.
fn pick_background(app: &mut PhotocraftApp) {
    let rgb = crate::color_picker_ui::parse_hex(&shown_background(app).1).unwrap_or([1.0; 3]);
    let title = crate::color_picker_ui::title_for(tl!("Color"));
    crate::color_picker_ui::open_for_target(app, "artboardBackground", &title, rgb);
}

/// Size, W, H, Make Portrait and Make Landscape. With a board selected they show and edit the
/// board (the returned `layer.artboard.set` params); with none, Size, W and H edit the tool's own
/// options and nothing is returned.
pub fn size_group(app: &mut PhotocraftApp, ui: &mut egui::Ui, board: Option<(LayerId, [i64; 4])>) -> Option<Value> {
    let t = Tokens::get(ui.ctx());
    let mut edit: Option<Value> = None;
    let (mut preset, mut w, mut h) = match board {
        Some((id, r)) => (board_preset(app, id), (r[2] - r[0]) as f32, (r[3] - r[1]) as f32),
        None => (app.ui.artboard_tool.preset.clone(), app.ui.artboard_tool.width as f32, app.ui.artboard_tool.height as f32),
    };
    let id = board.map_or(LayerId(0), |b| b.0);
    ui.label(egui::RichText::new(tl!("Size")).color(t.text_dim));
    let mut opts: Vec<(String, &str)> = vec![(String::new(), tl!("Custom"))];
    opts.extend(PRESETS.iter().map(|(n, _, _)| (n.to_string(), *n)));
    if crate::widgets::dropdown(ui, "artboard-tool-preset", tl!("Preset"), &mut preset, &opts, 150.0) && !preset.is_empty() {
        if board.is_some() {
            edit = Some(json!({"layer": id.0, "preset": preset}));
        } else {
            set_preset(app, &preset);
        }
    }
    ui.label(egui::RichText::new(tl!("W")).color(t.text_dim));
    let cw = crate::widgets::value_field(ui, &mut w, 1.0..=MAX_SIDE, "px", 64.0).accessible_name(tl!("Width")).changed();
    ui.label(egui::RichText::new(tl!("H")).color(t.text_dim));
    let ch = crate::widgets::value_field(ui, &mut h, 1.0..=MAX_SIDE, "px", 64.0).accessible_name(tl!("Height")).changed();
    if cw || ch {
        if board.is_some() {
            edit = Some(json!({"layer": id.0, "width": w.round(), "height": h.round(), "coalesce": format!("artboard-tool-wh-{}", id.0)}));
        } else {
            set_size(app, w, h);
        }
    }
    ui.add_enabled_ui(board.is_some(), |ui| {
        if crate::widgets::secondary_button(ui, tl!("Make Portrait"), 0.0).clicked() && w > h {
            edit = Some(json!({"layer": id.0, "width": h.round(), "height": w.round()}));
        }
        if crate::widgets::secondary_button(ui, tl!("Make Landscape"), 0.0).clicked() && h > w {
            edit = Some(json!({"layer": id.0, "width": h.round(), "height": w.round()}));
        }
    });
    edit
}

/// The background swatch and its dropdown (White, Black, Transparent, Other…). With boards selected
/// they show the first selected board's background and a change recolours every selected board in
/// one history step (`layer.artboard.set`; "Edit Artboard" for one, "Edit Artboards" for several);
/// either way it is also the tool option, the background of the next drawn board. With no board
/// selected it edits the tool option only.
pub fn background(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let (name, hex) = shown_background(app);
    if crate::widgets::color_swatch_button(ui, background_color(&name, &hex), tl!("Artboard Background Color")).clicked() {
        pick_background(app);
    }
    let mut bg = name;
    let opts = [
        ("white".to_string(), tl!("White")),
        ("black".to_string(), tl!("Black")),
        ("transparent".to_string(), tl!("Transparent")),
        ("custom".to_string(), tl!("Other…")),
    ];
    if crate::widgets::dropdown(ui, "artboard-tool-bg", tl!("Background"), &mut bg, &opts, 120.0) {
        if bg == "custom" {
            pick_background(app);
        } else if let Err(e) = set_background(app, &bg, &hex) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
}

/// The Align button with its panel, and the gear button with its menu.
pub fn align_and_gear(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    crate::widgets::vline(ui, 22.0);
    let align = icons::button_with_icon_size(ui, "align-edge-left", 28.0, 20.0, false, tl!("Align"));
    egui::Popup::menu(&align).show(|ui| {
        ui.set_min_width(200.0);
        for (g, group) in ALIGN_PANEL.iter().enumerate() {
            if g > 0 {
                ui.separator();
            }
            for (i, (label, cmd)) in group.iter().enumerate() {
                let on = cmd.is_some_and(|c| app.session.is_enabled(c));
                let icon = match cmd {
                    Some(c) => crate::align_bar::ALIGN.iter().find(|a| a.0 == *c).map_or("align-edge-left", |a| a.1),
                    None if *label == "Distribute Widths" => "distribute-horizontal",
                    None => "distribute-vertical",
                };
                let tint = if on { t.icon } else { t.text_dim };
                let button = egui::Button::image_and_text(icons::image(icon, 16.0, tint), tl!(label));
                if ui.add_enabled(on, button).clicked() {
                    if let Some(Err(e)) = run_align(app, g, i) {
                        app.ui.status = e;
                        app.ui.status_error = true;
                    }
                    ui.close();
                }
            }
        }
    });
    let gear = icons::button_with_icon_size(ui, "settings", 28.0, 20.0, false, tl!("Options"));
    egui::Popup::menu(&gear).show(|ui| {
        for (item, label) in GEAR {
            let mut on = item.checked(&app.ui.artboard_tool);
            if ui.add_enabled(item.enabled(), egui::Checkbox::new(&mut on, tl!(label))).changed() && item == Gear::AutoNest {
                app.ui.artboard_tool.auto_nest = on;
            }
        }
    });
}
