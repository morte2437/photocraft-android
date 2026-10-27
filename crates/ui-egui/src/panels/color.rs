use super::*;

/// The Color panel's foreground and background chips. A click picks the colour the field edits,
/// framed; a double-click opens the Color Picker on it.
pub(crate) fn field_chips(app: &mut PhotocraftApp, ui: &mut egui::Ui, chips: Rect) {
    let t = Tokens::get(ui.ctx());
    let bgr = Rect::from_min_size(chips.min + vec2(13.0, 13.0), vec2(22.0, 22.0));
    let fgr = Rect::from_min_size(chips.min + vec2(3.0, 3.0), vec2(22.0, 22.0));
    let frame = Stroke::new(1.0, t.text_dim);
    let bg_active = app.ui.color_panel.background;
    let p = ui.painter();
    p.rect_filled(bgr, 2.0, c32(app.session.tools.background));
    p.rect_stroke(bgr, 2.0, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
    if bg_active {
        p.rect_stroke(bgr.expand(2.0), 2.0, frame, StrokeKind::Outside);
    }
    p.rect_filled(fgr, 2.0, c32(app.session.tools.foreground));
    p.rect_stroke(fgr, 2.0, Stroke::new(1.0, Color32::from_gray(210)), StrokeKind::Outside);
    if !bg_active {
        p.rect_stroke(fgr.expand(2.0), 2.0, frame, StrokeKind::Outside);
    }
    // The foreground is on top, so it takes the clicks where the two overlap.
    let bg_resp = ui.interact(bgr, ui.scope_id().with("field-bg"), Sense::click());
    let fg_resp = ui.interact(fgr, ui.scope_id().with("field-fg"), Sense::click());
    let picked = if fg_resp.clicked() { false } else { bg_resp.clicked() || bg_active };
    app.ui.color_panel.background = picked;
    if fg_resp.double_clicked() || bg_resp.double_clicked() {
        crate::color_picker_ui::open(app, if picked { "background" } else { "foreground" });
    }
}

/// Where the dock tells the Color field how tall its group body is.
pub(crate) fn color_field_fill_id() -> egui::Id {
    egui::Id::unique("color-field-fill-height")
}

/// The Color field stretches to its dock group's height instead of a fixed 120 pt, in every theme
/// (#6000): a dock calls this with the group body's `ui` before drawing the tab.
pub(crate) fn offer_color_field_fill(ui: &egui::Ui, pro: bool, group: crate::dock::Group, tab: usize) {
    if group == crate::dock::Group::Color && group.tabs(pro).get(tab) == Some(&"Color") {
        let inner = ui.available_height().max(0.0);
        ui.data_mut(|d| d.insert_temp(color_field_fill_id(), inner));
    }
}

/// Height of the Color field: the group body's height (when docked) less the hex/RGB readout,
/// which folds onto a second line in a narrow dock. 120 pt when nothing sets a height.
pub(super) fn color_field_height(avail: Option<f32>, width: f32) -> f32 {
    let reserve = 32.0 + if width < 280.0 { 26.0 } else { 0.0 };
    // When docked, shrink the field so the hex/RGB readout stays in view (#6885). A 120 pt floor
    // here pushed the readout below the scroll area in a compact Color group.
    avail.filter(|a| a.is_finite()).map_or(120.0, |a| (a - reserve).clamp(0.0, 2000.0))
}

/// The colour the SV field draws for `hsva`, with what the field showed last (`hue`, `saturation`)
/// filling in what the colour itself cannot carry: a grey has no hue of its own, and black has
/// neither hue nor saturation — so a drag that bottoms out into black would otherwise round-trip
/// through RGB as `s = 0` and drop the knob from the bottom-right corner to the bottom-left one
/// (#3228).
pub(crate) fn field_hsva(mut hsva: egui::ecolor::Hsva, remembered: (f32, f32)) -> egui::ecolor::Hsva {
    if hsva.v < 0.01 {
        hsva.h = remembered.0;
        hsva.s = remembered.1;
    } else if hsva.s < 0.01 {
        // A grey: the saturation really is zero (that is where the knob belongs), only the hue
        // is undefined.
        hsva.h = remembered.0;
    }
    hsva
}

/// Photoshop's Color panel Hue Cube (saturation/brightness field + hue strip) or Brightness Cube
/// (hue/saturation field + brightness strip), drawn as shaded meshes, over a hex/RGB readout.
pub(crate) fn color_field(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let bg_active = app.ui.color_panel.background;
    let key = egui::Id::unique(("color-field-hue", bg_active));
    let mut hsva = srgb_hsva(if bg_active { app.session.tools.background } else { app.session.tools.foreground });
    // What the field showed last: a grey carries no hue, black neither hue nor saturation (#3228).
    let remembered: (f32, f32) = ui.data(|d| d.get_temp(key)).unwrap_or((hsva.h, hsva.s));
    hsva = field_hsva(hsva, remembered);
    let w = ui.available_width();
    let strip_w = 14.0;
    // Photoshop's Color panel field fills the panel in every theme (#6000): take the height the
    // dock offers, less the hex/RGB readout (which folds onto a second line in a narrow dock) and
    // the theme's spacing between the rows. Without a dock, 120 pt.
    let gaps = 2.0 * ui.spacing().item_spacing.y;
    let fill = ui.data(|d| d.get_temp::<f32>(color_field_fill_id())).map(|a| a - gaps);
    let h = color_field_height(fill, w);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (chips, _) = ui.allocate_exact_size(vec2(38.0, h), Sense::hover());
        field_chips(app, ui, chips);
        // Hue Cube: saturation across and brightness up, hue on the strip. Brightness Cube (the
        // Color Picker's B mode): hue across and saturation up, brightness on the strip.
        let brightness = app.ui.color_panel.mode == crate::state::ColorPanelMode::BrightnessCube;
        let field_w = w - 38.0 - strip_w - 16.0;
        let (field, fresp) = ui.allocate_exact_size(vec2(field_w, h), Sense::click_and_drag());
        let at = |x: f32, y: f32| {
            if brightness { egui::ecolor::Hsva::new(x.min(0.9999), 1.0 - y, hsva.v, 1.0) } else { egui::ecolor::Hsva::new(hsva.h, x, 1.0 - y, 1.0) }
        };
        ui.painter().add(color_grid(field, 16, |x, y| hsva_color32(at(x, y))));
        ui.painter().rect_stroke(field, 0.0, Stroke::new(1.0, t.separator), StrokeKind::Outside);
        let field_pick = widgets::pick_pos(&fresp);
        if let Some(p) = field_pick {
            let x = ((p.x - field.left()) / field.width()).clamp(0.0, 1.0);
            let y = ((p.y - field.top()) / field.height()).clamp(0.0, 1.0);
            if brightness {
                hsva.h = x.min(0.9999);
                hsva.s = 1.0 - y;
            } else {
                hsva.s = x;
                hsva.v = 1.0 - y;
            }
        }
        let (kx, ky) = if brightness { (hsva.h, 1.0 - hsva.s) } else { (hsva.s, 1.0 - hsva.v) };
        let knob = pos2(field.left() + kx * field.width(), field.top() + ky * field.height());
        ui.painter().circle_stroke(knob, 5.0, Stroke::new(1.5, Color32::WHITE));
        ui.painter().circle_stroke(knob, 6.5, Stroke::new(1.0, Color32::from_black_alpha(160)));
        // The strip: hue (red at both ends, as Photoshop's) or brightness (the colour down to black).
        let (strip, sresp) = ui.allocate_exact_size(vec2(strip_w, h), Sense::click_and_drag());
        let strip_at = |f: f32| {
            if brightness { egui::ecolor::Hsva::new(hsva.h, hsva.s, 1.0 - f, 1.0) } else { egui::ecolor::Hsva::new(1.0 - f, 1.0, 1.0, 1.0) }
        };
        ui.painter().add(color_grid(strip, 24, |_, y| hsva_color32(strip_at(y))));
        let strip_pick = widgets::pick_pos(&sresp);
        if let Some(p) = strip_pick {
            let f = ((p.y - strip.top()) / strip.height()).clamp(0.0, 1.0);
            if brightness {
                hsva.v = 1.0 - f;
            } else {
                hsva.h = 1.0 - f.min(0.9999);
            }
        }
        let y = strip.top() + (1.0 - if brightness { hsva.v } else { hsva.h }) * strip.height();
        let tri = vec![pos2(strip.right() + 1.0, y), pos2(strip.right() + 6.0, y - 4.0), pos2(strip.right() + 6.0, y + 4.0)];
        ui.painter().add(egui::Shape::convex_polygon(tri, t.text, Stroke::NONE));
        if field_pick.is_some() || strip_pick.is_some() {
            if bg_active {
                app.session.tools.background = hsva_srgb(hsva);
            } else if app.session.tools.foreground != hsva_srgb(hsva) {
                // A held, unmoved press repeats the pick: restyle selected type only on a change.
                app.session.tools.foreground = hsva_srgb(hsva);
                crate::apply_color::foreground_changed(app, crate::apply_color::gesture(ui.ctx()));
            }
            ui.data_mut(|d| d.insert_temp(key, (hsva.h, hsva.s)));
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let mut c = if bg_active { app.session.tools.background } else { app.session.tools.foreground };
        if color_readout(ui, ui.scope_id().with(("color-field", bg_active)), &mut c) {
            if bg_active {
                app.session.tools.background = c;
            } else {
                app.session.tools.foreground = c;
                crate::apply_color::foreground_changed(app, crate::apply_color::gesture(ui.ctx()));
            }
            // Keep the hue for greys, so the field marker doesn't jump to red; the saturation with
            // it, so a later black lands on the corner it was last dragged to (#3228).
            let h = srgb_hsva(c);
            if h.s >= 0.01 && h.v >= 0.01 {
                ui.data_mut(|d| d.insert_temp(key, (h.h, h.s)));
            }
        }
    });
    crate::apply_color::track_focus(ui.ctx());
}

/// A smooth-shaded `n` x `n` grid over `rect`, coloured by `color(x, y)` with x and y in 0..1
/// (y down).
pub(crate) fn color_grid(rect: Rect, n: usize, color: impl Fn(f32, f32) -> Color32) -> egui::Mesh {
    let n = n.max(1);
    let mut mesh = egui::Mesh::default();
    for j in 0..=n {
        for i in 0..=n {
            let (x, y) = (i as f32 / n as f32, j as f32 / n as f32);
            mesh.colored_vertex(pos2(rect.left() + x * rect.width(), rect.top() + y * rect.height()), color(x, y));
        }
    }
    let row = (n + 1) as u32;
    for j in 0..n as u32 {
        for i in 0..n as u32 {
            let a = j * row + i;
            mesh.add_triangle(a, a + 1, a + row + 1);
            mesh.add_triangle(a, a + row + 1, a + row);
        }
    }
    mesh
}

/// Tool colours are sRGB-encoded floats; egui's Hsva works on sRGB bytes via these helpers
/// (its `from_rgba_unmultiplied` expects *linear* RGB, which gave wrong readouts).
pub(crate) fn srgb_bytes(c: [f32; 4]) -> [u8; 3] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [q(c[0]), q(c[1]), q(c[2])]
}

/// The Color panel's HSB, as Photoshop's and the Color Picker's: on the sRGB-encoded values,
/// so a 50% grey has Brightness 50%. egui's `Hsva` conversions work in linear light, which put
/// that grey at 22% and drew the field's middle as 188 (#3323); `Hsva` only holds the numbers.
pub(crate) fn srgb_hsva(c: [f32; 4]) -> egui::ecolor::Hsva {
    let [h, s, v] = photocraft_color::convert::rgb_to_hsb([c[0], c[1], c[2]].map(|v| v.clamp(0.0, 1.0)));
    egui::ecolor::Hsva::new(h / 360.0, s, v, 1.0)
}

/// The inverse of srgb_hsva: the Color panel's HSB back to sRGB-encoded values.
pub(crate) fn hsva_srgb(h: egui::ecolor::Hsva) -> [f32; 4] {
    let [r, g, b] = photocraft_color::convert::hsb_to_rgb([h.h * 360.0, h.s.clamp(0.0, 1.0), h.v.clamp(0.0, 1.0)]);
    [r, g, b, 1.0]
}

/// The colour to draw for Color panel HSB `h` (see [`srgb_hsva`]).
pub(crate) fn hsva_color32(h: egui::ecolor::Hsva) -> Color32 {
    let [r, g, b] = srgb_bytes(hsva_srgb(h));
    Color32::from_rgb(r, g, b)
}

/// The Color panel's editable colour readout: a hex field and R, G, B fields, all drawn like the
/// dock's [`widgets::value_field`] so they sit in the theme. Editing any of them sets `color`
/// (sRGB-encoded floats) and returns `true`.
pub(crate) fn color_readout(ui: &mut egui::Ui, id: egui::Id, color: &mut [f32; 4]) -> bool {
    let t = Tokens::get(ui.ctx());
    let mut changed = false;
    // Wrapped so the fields fold onto a second line in a narrow dock instead of being clipped.
    ui.horizontal_wrapped(|ui| {
        // Docks zero the item spacing; keep the fields apart.
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new("#").font(theme::mono(12.0)).color(t.text_faint));
        changed = hex_edit(ui, id.with("hex"), color);
        let [mut r, mut g, mut b] = srgb_bytes(*color).map(f32::from);
        let mut rgb = false;
        for (label, name, v) in [("R", tl!("Red"), &mut r), ("G", tl!("Green"), &mut g), ("B", tl!("Blue"), &mut b)] {
            ui.label(RichText::new(label).font(theme::mono(11.0)).color(t.text_faint));
            rgb |= widgets::value_field(ui, v, 0.0..=255.0, "", 38.0).accessible_name(name).changed();
        }
        if rgb {
            *color = [r / 255.0, g / 255.0, b / 255.0, 1.0];
            changed = true;
        }
    });
    changed
}

/// The hex field of [`color_readout`]: type a colour with or without the leading `#`. Valid input
/// sets `color` and returns `true`. While it has focus it shows exactly what is typed, so partial
/// or invalid input isn't overwritten by the colour; an invalid entry snaps back when focus leaves.
fn hex_edit(ui: &mut egui::Ui, id: egui::Id, color: &mut [f32; 4]) -> bool {
    let t = Tokens::get(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(64.0, 24.0), Sense::hover());
    widgets::surface(ui, rect, t.field, false);
    if !t.bevel {
        ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    }
    let [r, g, b] = srgb_bytes(*color);
    let typing = if ui.memory(|m| m.has_focus(id)) { ui.data(|d| d.get_temp::<String>(id)) } else { None };
    let mut text = typing.unwrap_or_else(|| format!("{r:02X}{g:02X}{b:02X}"));
    let field = rect.shrink2(vec2(5.0, 2.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(field).layout(egui::Layout::left_to_right(egui::Align::Center)));
    // Room for a pasted "#rrggbb"; the parse trims it, and the field shows plain digits otherwise.
    // `Frame::NONE`: the themed surface above is this field's frame.
    let resp = child
        .add(egui::TextEdit::singleline(&mut text).id(id).char_limit(7).desired_width(field.width()).frame(egui::Frame::NONE).font(theme::mono(12.0)))
        .accessible_name(tl!("Color"));
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    if resp.changed()
        && let Some(c) = crate::color_picker_ui::parse_hex(&text)
    {
        *color = [c[0], c[1], c[2], 1.0];
        return true;
    }
    false
}
