//! UI for the Eraser group's colour-aware tools: Magic Eraser (click → `paint.magicEraser`) and
//! Background Eraser (stroke → `paint.backgroundEraser`), with their options bars.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::Tokens;
use crate::{icons, widgets};

fn report(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Magic Eraser click at document `(x, y)`. Returns false for other tools.
pub fn click(app: &mut PhotocraftApp, tool: Tool, x: f64, y: f64) -> bool {
    if tool != Tool::MagicEraser {
        return false;
    }
    let o = &app.ui.tool_options;
    let p = json!({
        "x": x.floor(),
        "y": y.floor(),
        "tolerance": o.tolerance,
        "antiAlias": o.anti_alias,
        "contiguous": o.contiguous,
        "sampleAllLayers": o.sample_all.magic_eraser,
        "opacity": o.magic_eraser_opacity,
    });
    let r = app.run("paint.magicEraser", p);
    report(app, r);
    true
}

/// Finishes a Background Eraser stroke. Returns false for other tools.
pub fn finish_stroke(app: &mut PhotocraftApp, tool: Tool, points: &[[f64; 3]], line: bool) -> bool {
    if tool != Tool::BackgroundEraser {
        return false;
    }
    let mut p = background_eraser_params(app, points);
    crate::canvas::unsmoothed_line(&mut p, line);
    let r = app.run("paint.backgroundEraser", p);
    report(app, r);
    true
}

/// `paint.backgroundEraser` params for a stroke along `points`: shared by the live preview and
/// the commit, with the jitter seed from the first point (`retouch_ui::stroke_seed`).
pub(crate) fn background_eraser_params(app: &PhotocraftApp, points: &[[f64; 3]]) -> Value {
    let o = &app.ui.tool_options;
    json!({
        "points": points,
        "sampling": o.bg_sampling,
        "limits": o.bg_limits,
        "tolerance": stored_tolerance(o.bg_tolerance),
        "protectForegroundColor": o.bg_protect_fg,
        "seed": crate::retouch_ui::stroke_seed(points),
    })
}

/// Whether `tool` is the Eraser in Pencil mode: it erases hard, aliased pixels through
/// `paint.pencil` (`"erase": true`) and shows the Pencil's pixel-grid cursor (#2662).
pub fn pencil_mode(app: &PhotocraftApp, tool: Tool) -> bool {
    tool == Tool::Eraser && app.ui.tool_options.eraser_mode == "pencil"
}

/// Whether `tool` is the Eraser in Block mode: it erases a hard square of 16 screen pixels
/// (`16 / zoom` document pixels) through `paint.pencil` (`"block": true`); Size, Hardness,
/// Opacity and Flow don't apply (#2770).
pub fn block_mode(app: &PhotocraftApp, tool: Tool) -> bool {
    tool == Tool::Eraser && app.ui.tool_options.eraser_mode == "block"
}

/// The Block's edge in document pixels at the current zoom (what the engine erases).
pub fn block_size(app: &PhotocraftApp) -> f32 {
    photocraft_engine::brush_cmds::block_size(f64::from(app.point_zoom()))
}

/// The Eraser's Mode dropdown (Brush / Pencil / Block). Returns whether it changed.
pub fn mode_dropdown(ui: &mut egui::Ui, mode: &mut String) -> bool {
    opt(ui, tl!("Mode:"));
    let modes = [("brush".to_string(), tl!("Brush")), ("pencil".to_string(), tl!("Pencil")), ("block".to_string(), tl!("Block"))];
    widgets::dropdown(ui, "eraser-mode", tl!("Mode"), mode, &modes, 96.0)
}

fn opt(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    let label = ui.label(egui::RichText::new(tl!(&text)).color(t.text_dim).size(12.0));
    crate::widgets::scrub_option_label(ui, &label);
}

/// The reference application's alert for a Background Eraser Angle typed outside −180…180.
pub(crate) const ANGLE_RANGE_MESSAGE: &str = "An integer between -180 ° and 180 ° is required. Closest value inserted.";

/// The Background Eraser's Angle: an angle icon and a whole-degree field on the brush tip's
/// angle, −180…180, default 0. Returns true when a typed value was out of range and was clamped
/// (400 → 180): the caller shows [`ANGLE_RANGE_MESSAGE`] once the entry is committed.
fn angle_field(ui: &mut egui::Ui, b: &mut photocraft_engine::BrushSettings) -> bool {
    let t = Tokens::get(ui.ctx());
    let tip = tl!("Set the brush angle");
    ui.add(icons::image("rotate-cw", 15.0, t.text_dim)).on_hover_text(tip);
    // A wider range than the angle's, so a typed 400 reaches us and can be told apart from 180.
    let mut a = b.angle;
    // Read before the field: it consumes the arrow keys it steps with, and the wheel it steps
    // with while focused never shows in the scroll delta (a view's scroll does, and is no step).
    let arrow_or_wheel =
        ui.input(|i| i.key_pressed(egui::Key::ArrowUp) || i.key_pressed(egui::Key::ArrowDown)) || crate::field_wheel::focused_field_wheeled(ui.ctx());
    let r = widgets::value_field_named(ui, tl!("Angle"), &mut a, -ANGLE_TYPED_MAX..=ANGLE_TYPED_MAX, "°", 52.0);
    let out_of_range = r.id.with("bg-eraser-angle-out-of-range");
    // The flag lives only while an entry is being typed: drawn without the keyboard, the field
    // drops it, so an entry the tool switch cut off (never committed) can't alert on a later one.
    // The widget's own focus, not `has_focus()`: that is false while the window is in the
    // background, and switching away mid-entry must keep the alert for the commit.
    if !ui.memory(|m| m.has_focus(r.id)) && !r.lost_focus() {
        ui.data_mut(|d| d.remove_temp::<bool>(out_of_range));
    }
    if r.changed() && a.is_finite() {
        let whole = a.trunc();
        let clamped = whole.clamp(-180.0, 180.0);
        b.angle = clamped;
        // Only typing alerts: a scrub or an arrow-key step past the end just stops there.
        let stepped = r.dragged() || arrow_or_wheel;
        // A wheel step over the field without its focus is no entry either.
        let typed_out = whole != clamped && !stepped && (r.has_focus() || r.lost_focus());
        ui.data_mut(|d| d.insert_temp(out_of_range, typed_out));
    }
    let r = r.on_hover_text(tip);
    // Enter (or a click away) commits the entry: the alert comes then, not mid-typing.
    r.lost_focus() && ui.data_mut(|d| d.remove_temp::<bool>(out_of_range)).unwrap_or(false)
}

/// How far the Angle field lets a typed number through before [`angle_field`] clamps it.
const ANGLE_TYPED_MAX: f32 = 1.0e6;

/// Options bar for the Magic Eraser and Background Eraser. Returns false for other tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    let o = &mut app.ui.tool_options;
    match tool {
        Tool::MagicEraser => {
            opt(ui, tl!("Tolerance:"));
            widgets::value_field(ui, &mut o.tolerance, 0.0..=255.0, "", 58.0);
            widgets::checkbox(ui, &mut o.anti_alias, tl!("Anti-alias"));
            widgets::checkbox(ui, &mut o.contiguous, tl!("Contiguous"));
            widgets::checkbox(ui, &mut o.sample_all.magic_eraser, tl!("Sample All Layers"));
            widgets::vline(ui, 22.0);
            opt(ui, tl!("Opacity:"));
            widgets::value_field(ui, &mut o.magic_eraser_opacity, 1.0..=100.0, "%", 62.0);
        }
        Tool::BackgroundEraser => {
            // Sampling: Continuous / Once / Background Swatch, as icon toggles (Photoshop).
            ui.spacing_mut().item_spacing.x = 2.0;
            for (k, icon, tip) in [
                ("continuous", "pipette", tl!("Sampling: Continuous")),
                ("once", "circle-dot", tl!("Sampling: Once")),
                ("backgroundSwatch", "square", tl!("Sampling: Background Swatch")),
            ] {
                if icons::button(ui, icon, 24.0, o.bg_sampling == k, tip).clicked() {
                    o.bg_sampling = k.into();
                }
            }
            ui.spacing_mut().item_spacing.x = 8.0;
            widgets::vline(ui, 22.0);
            opt(ui, tl!("Limits:"));
            let limits = [
                ("discontiguous".to_string(), tl!("Discontiguous")),
                ("contiguous".to_string(), tl!("Contiguous")),
                ("findEdges".to_string(), tl!("Find Edges")),
            ];
            widgets::dropdown(ui, "bg-eraser-limits", tl!("Limits:"), &mut o.bg_limits, &limits, 110.0);
            // The dropdown's button, by the id its combo box takes from "bg-eraser-limits".
            if let Some(limits) = ui.ctx().read_response(ui.make_persistent_id(egui::IdSalt::new("bg-eraser-limits"))) {
                limits.on_hover_text(tl!("How far to let the erasing spread"));
            }
            opt(ui, tl!("Tolerance:"));
            if tolerance_field(ui, &mut o.bg_tolerance) {
                open_range_alert(app);
            }
            // Angle and Pressure for Size edit the tool's own brush (the shell swaps one in per
            // tool), through `tools.setBrush` like the Brush Settings panel.
            let before = app.session.tools.brush.clone();
            let mut brush = before.clone();
            if angle_field(ui, &mut brush) {
                let message = tl!(ANGLE_RANGE_MESSAGE).to_string();
                app.ui.open_dialog(crate::state::DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
            }
            widgets::checkbox(ui, &mut app.ui.tool_options.bg_protect_fg, tl!("Protect Foreground Color"))
                .on_hover_text(tl!("Do not erase the foreground swatch color"));
            let name = tl!("Always use pressure for size");
            let pressure = icons::button_with_icon_size_tip(ui, "circle-dot", 24.0, (24.0f32 * 0.52).round(), brush.pressure_size, name, false);
            if pressure.on_hover_text(tl!("Always use Pressure for Size. When off, Brush Preset controls pressure")).clicked() {
                brush.pressure_size = !brush.pressure_size;
            }
            crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &brush);
        }
        _ => return false,
    }
    true
}

/// The alert for a Background Eraser Tolerance typed out of range (#11909).
pub const TOLERANCE_RANGE_ALERT: &str = "An integer between 1% and 100% is required. Closest value inserted.";

/// The Background Eraser's Tolerance: whole numbers in 1..=100, as the reference application takes
/// them (#11909). A typed fraction becomes the nearest whole number, a value past the range the
/// closest one. Returns true when the field is left with a value typed out of range, so the
/// caller shows [`TOLERANCE_RANGE_ALERT`].
fn tolerance_field(ui: &mut egui::Ui, value: &mut f32) -> bool {
    whole_tolerance_field(ui, value, "bg-eraser-tolerance-typed", tl!("Erase range of similar colors"))
}

/// [`tolerance_field`]'s field for any tool's whole-number Tolerance (the Color Replacement tool's
/// too, #11930): `draft` keys its typed-but-not-left text, `hover` is its tooltip.
pub(crate) fn whole_tolerance_field(ui: &mut egui::Ui, value: &mut f32, draft: &str, hover: &str) -> bool {
    // A restored setting (an older 0, a fraction) shows and erases as the field would store it.
    *value = stored_tolerance(*value);
    // While the field has the keyboard it gets back what was typed, not the stored value, so
    // its text is left alone mid-entry (`0` on the way to `05` must not turn into `1`).
    let draft_id = ui.make_persistent_id(egui::IdSalt::new(draft));
    // The field's own range is wide, so the typed value reaches here unclamped.
    // A draft only stands while the setting is still the one it stored: a value set elsewhere
    // meanwhile (a tool preset while the bar was hidden) wins.
    let draft = ui.ctx().data(|d| d.get_temp::<(f32, f32)>(draft_id));
    let mut typed = draft.filter(|(_, stored)| *stored == *value).map_or(*value, |(typed, _)| typed);
    let r = widgets::value_field(ui, &mut typed, -1.0e9..=1.0e9, "%", 62.0).on_hover_text(hover);
    let out_id = r.id.with("out-of-range");
    // An edit the field never got to finish (the options bar was hidden while it had the
    // keyboard) has nothing left to say once the field is entered again.
    if r.gained_focus() {
        ui.ctx().data_mut(|d| {
            d.remove_temp::<bool>(out_id);
            d.remove_temp::<bool>(r.id.with("parses"));
        });
    }
    if r.changed() && typed.is_finite() {
        *value = typed.round().clamp(1.0, 100.0);
        // Only what was typed alerts, also an expression committed as the field is left
        // (`50*4` and Enter); a scrub past the ends (the field's or its caption's, even while
        // the field has the keyboard) just stops there.
        let typing = (r.has_focus() || r.lost_focus()) && ui.ctx().dragged_id().is_none();
        let out = typing && !(1.0..=100.0).contains(&typed);
        ui.ctx().data_mut(|d| d.insert_temp(out_id, out));
        if typing && r.has_focus() {
            ui.ctx().data_mut(|d| d.insert_temp(draft_id, (typed, *value)));
        } else {
            ui.ctx().data_mut(|d| d.remove_temp::<(f32, f32)>(draft_id));
        }
    }
    // Text replaced by something that is no number (`150`, then `abc`) leaves the value alone
    // when the field is left, with no alert (#12268). A number typed again after it (`150` once
    // more) owes the alert again, even though the value did not change.
    let no_number_id = r.id.with("no-number-typed");
    if r.has_focus() {
        match widgets::typed_text_parses(ui.ctx(), r.id) {
            Some(false) => ui.ctx().data_mut(|d| {
                d.remove_temp::<bool>(out_id);
                d.insert_temp(no_number_id, true);
            }),
            Some(true) if ui.ctx().data_mut(|d| d.remove_temp::<bool>(no_number_id)).unwrap_or(false) => {
                let out = typed.is_finite() && !(1.0..=100.0).contains(&typed);
                ui.ctx().data_mut(|d| d.insert_temp(out_id, out));
            }
            _ => {}
        }
    } else {
        ui.ctx().data_mut(|d| d.remove_temp::<bool>(no_number_id));
    }
    if !r.has_focus() {
        ui.ctx().data_mut(|d| d.remove_temp::<(f32, f32)>(draft_id));
    }
    let alert = r.lost_focus() && ui.ctx().data_mut(|d| d.remove_temp::<bool>(out_id)).unwrap_or(false);
    // While a value typed out of range waits for the field to be left, the bar notes the pass, so
    // a field that goes away instead (a tool shortcut that passes the field by) still alerts.
    let owed = r.has_focus() && ui.ctx().data(|d| d.get_temp::<bool>(out_id)).unwrap_or(false);
    let pass = ui.ctx().cumulative_pass_nr();
    ui.ctx().data_mut(|d| {
        if owed {
            d.insert_temp(range_alert_owed_id(), pass);
        } else {
            d.remove_temp::<u64>(range_alert_owed_id());
        }
    });
    alert
}

/// Where a whole-number Tolerance field notes the pass in which it last owed the range alert.
fn range_alert_owed_id() -> egui::Id {
    egui::Id::unique("whole-tolerance-range-alert-owed")
}

/// Shows [`TOLERANCE_RANGE_ALERT`] for a whole-number Tolerance field that went away in the last
/// pass while it held a value typed out of range (the tool changed by a shortcut, #12268). The
/// options bar calls it before drawing a tool's options. An edit older than that (the bar was
/// hidden meanwhile) is dropped, as the field drops it once entered again.
pub(crate) fn alert_for_a_field_gone(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(seen) = ctx.data(|d| d.get_temp::<u64>(range_alert_owed_id())) else { return };
    let now = ctx.cumulative_pass_nr();
    if seen.saturating_add(1) >= now {
        return;
    }
    ctx.data_mut(|d| d.remove_temp::<u64>(range_alert_owed_id()));
    if seen.saturating_add(2) == now {
        open_range_alert(app);
    }
}

/// A Tolerance as the field stores it: a whole number in 1..=100, the default 50 for a value
/// that is no number. Strokes use it too, so a restored setting erases the same before and after
/// the options bar shows it.
pub(crate) fn stored_tolerance(v: f32) -> f32 {
    if v.is_finite() { v.round().clamp(1.0, 100.0) } else { 50.0 }
}

/// Shows [`TOLERANCE_RANGE_ALERT`]: a modal with OK, once even if more than one pass asks.
pub(crate) fn open_range_alert(app: &mut PhotocraftApp) {
    let message = tl!(TOLERANCE_RANGE_ALERT).to_owned();
    let shown = app.ui.dialogs.iter().any(|d| d.kind == crate::state::DialogKind::Error && d.fields.get("message").and_then(|m| m.as_str()) == Some(&message));
    if !shown {
        app.ui.open_dialog(crate::state::DialogKind::Error, json!({ "message": message }).as_object().cloned().unwrap_or_default());
    }
}
