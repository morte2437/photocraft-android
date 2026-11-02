use super::{Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Tokens, Ui, Vec2, pos2};

/// Cached RGB data; drawing work is bounded by the 256 display bins, never the image size.
pub fn rgb_histogram(ui: &mut Ui, histogram: &photocraft_algo::histogram::RgbHistogram, height: f32) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width().max(1.0), height.max(1.0)), egui::Sense::hover());
    ui.painter().rect_filled(rect, t.radius_sm, t.histogram_background());
    let plot = rect.shrink(4.0);
    crate::rgb_histogram::paint(ui.painter(), plot, histogram, &t);
    ui.painter().rect_stroke(rect, t.radius_sm, egui::Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    response
}

/// The fully saturated colour at hue `deg` (degrees), built on sRGB values as Photoshop's hue
/// tracks are: 0 is (255,0,0), 60 is (255,255,0), 120 is (0,255,0) and so on (#6540).
pub fn hue_color(deg: f32) -> Color32 {
    let [r, g, b] = photocraft_color::convert::hsb_to_rgb([deg, 1.0, 1.0]);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(r), q(g), q(b))
}

/// Hue spectrum stops for colour sliders: red, yellow, green, cyan, blue, magenta, red.
pub fn hue_stops() -> Vec<Color32> {
    (0..=12).map(|i| hue_color(i as f32 * 30.0)).collect()
}

/// Paint a small checkerboard (transparency) in `rect`.
pub fn checker(painter: &egui::Painter, rect: Rect, cell: f32) {
    painter.rect_filled(rect, 0.0, Color32::from_gray(250));
    let nx = (rect.width() / cell).ceil() as i32;
    let ny = (rect.height() / cell).ceil() as i32;
    for j in 0..ny {
        for i in 0..nx {
            if (i + j) % 2 == 1 {
                let r = Rect::from_min_size(Pos2::new(rect.left() + i as f32 * cell, rect.top() + j as f32 * cell), Vec2::splat(cell)).intersect(rect);
                painter.rect_filled(r, 0.0, Color32::from_gray(214));
            }
        }
    }
}

/// A transparency checkerboard of `cell`-point squares clipped to `rect` with `radius` corners,
/// for rounded swatches (#3208): [`checker`] paints square cells, which showed past a rounded
/// fill's corners. This one is a repeating two-by-two tile textured onto the rounded rect.
pub fn checker_rounded(painter: &egui::Painter, rect: Rect, cell: f32, radius: impl Into<CornerRadius>) {
    let tile = checker_tile(painter.ctx());
    painter.add(egui::epaint::RectShape::filled(rect, radius, Color32::WHITE).with_texture(tile.id(), checker_uv(rect, cell)));
}

/// The texture range that tiles `rect` with `cell`-point squares: the tile holds two cells a side.
pub(super) fn checker_uv(rect: Rect, cell: f32) -> Rect {
    let cell = if cell.is_finite() && cell > 0.0 { cell } else { 4.0 };
    Rect::from_min_max(Pos2::ZERO, pos2(rect.width() / (2.0 * cell), rect.height() / (2.0 * cell)))
}

pub(super) fn checker_tile_id() -> egui::Id {
    egui::Id::unique("transparency-checker-tile")
}

/// The checker's two-by-two tile (light, dark / dark, light), loaded once per context and shared
/// by every rounded checker; it repeats and never blurs.
fn checker_tile(ctx: &egui::Context) -> egui::TextureHandle {
    if let Some(tile) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(checker_tile_id())) {
        return tile;
    }
    let (light, dark) = (Color32::from_gray(250), Color32::from_gray(214));
    let image = egui::ColorImage::new([2, 2], vec![light, dark, dark, light]);
    let options = egui::TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Nearest,
        wrap_mode: egui::TextureWrapMode::Repeat,
        mipmap_mode: None,
    };
    let tile = ctx.load_texture("transparency-checker-tile", image, options);
    ctx.data_mut(|d| d.insert_temp(checker_tile_id(), tile.clone()));
    tile
}

/// A colour swatch that opens PhotoCraft's Color Picker dialog (HSB, RGB, Lab, CMYK, hex and the
/// eyedropper), titled `title` as Photoshop's "Color Picker (…)", on its colour. While it is open
/// the swatch previews the colour it shows; OK keeps the colour (the alpha stays), Cancel restores
/// the exact colour the swatch had. The response is `changed` in every frame the colour changes;
/// [`crate::color_picker_ui::widget_picker_open`] tells a preview from the final colour.
pub fn color_edit_button_srgba(ui: &mut Ui, color: &mut Color32, title: &str) -> Response {
    color_edit_button(ui, color, title, false)
}

pub(crate) fn color_swatch(ui: &mut Ui, color: Color32) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(ui.spacing().interact_size, Sense::click());
    // A swatch's meaning is the colour it edits; callers with a more specific caption use
    // `color_swatch_button` or add `labelled_by`.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColorWell, ui.is_enabled(), tl!("Color")));
    checker_rounded(ui.painter(), rect, 4.0, t.radius_sm);
    ui.painter().rect_filled(rect.shrink(1.0), t.radius_sm, color);
    ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    response
}

/// Where a press or drag on a picking surface (a colour field, a hue strip) points, from the
/// frame the button goes down and on every move after it. `dragged()` waits for egui's click
/// threshold (a few points of travel, or a long hold), so a press picked nothing until the
/// pointer had moved away and fine adjustments jumped (#7793). Only the primary button picks:
/// `is_pointer_button_down_on` is true for any button, so a right or middle press is ignored.
pub fn pick_pos(resp: &Response) -> Option<Pos2> {
    let held = resp.is_pointer_button_down_on()
        && resp.ctx.input(|i| i.pointer.primary_down() && !i.pointer.button_down(egui::PointerButton::Secondary) && !i.pointer.middle_down());
    if held || resp.clicked() { resp.interact_pointer_pos() } else { None }
}

/// A colour swatch that only reports clicks (no popup): for colours edited in PhotoCraft's own
/// Color Picker dialog. `label` names it for hover text and accessibility.
pub fn color_swatch_button(ui: &mut Ui, color: Color32, label: &str) -> Response {
    let t = Tokens::get(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(ui.spacing().interact_size, Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ColorWell, true, label));
    checker_rounded(ui.painter(), rect, 4.0, t.radius_sm);
    ui.painter().rect_filled(rect.shrink(1.0), t.radius_sm, color);
    ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), StrokeKind::Inside);
    response.on_hover_text(label)
}

/// Where a swatch keeps the exact colour it had when it opened its Color Picker, for Cancel.
fn opening_id(response: &Response) -> egui::Id {
    response.id.with("picker-opening")
}

/// Set once the open picker previewed a colour on the swatch.
fn previewed_id(response: &Response) -> egui::Id {
    response.id.with("picker-previewed")
}

/// The swatch was clicked: keep its exact colour for Cancel.
fn stash_opening<T: Clone + Send + Sync + 'static>(ui: &Ui, response: &Response, value: &T) {
    ui.data_mut(|d| {
        d.insert_temp(opening_id(response), value.clone());
        d.remove::<bool>(previewed_id(response));
    });
}

/// Follow the swatch's Color Picker: OK's colour (`picked`), the colour it shows while open, or
/// the colour the swatch opened it with when it closes without one. True in the frame a preview
/// changes `value` and in the frame the picker closes after OK or a preview, so a host can commit
/// then; a picker closed without either changes nothing (a host must not rewrite its values).
fn follow_picker<T: Clone + PartialEq + Send + Sync + 'static>(ui: &Ui, response: &Response, value: &mut T, picked: impl Fn([f32; 3], &T) -> T) -> bool {
    use crate::color_picker_ui::{WidgetEvent, take_widget_color, take_widget_event};
    let ctx = ui.ctx();
    if let Some(rgb) = take_widget_color(ctx, response.id) {
        ctx.data_mut(|d| {
            d.remove::<T>(opening_id(response));
            d.remove::<bool>(previewed_id(response));
        });
        *value = picked(rgb, value);
        return true;
    }
    match take_widget_event(ctx, response.id) {
        Some(WidgetEvent::Preview(rgb)) => {
            let v = picked(rgb, value);
            let changed = v != *value;
            if changed {
                ctx.data_mut(|d| d.insert_temp(previewed_id(response), true));
            }
            *value = v;
            changed
        }
        Some(WidgetEvent::Restore) => {
            let (opening, previewed) = ctx.data_mut(|d| {
                let v = d.get_temp::<T>(opening_id(response));
                let previewed = d.get_temp::<bool>(previewed_id(response)) == Some(true);
                d.remove::<T>(opening_id(response));
                d.remove::<bool>(previewed_id(response));
                (v, previewed)
            });
            let Some(v) = opening else { return false };
            let changed = previewed || v != *value;
            *value = v;
            changed
        }
        None => false,
    }
}

fn color_edit_button(ui: &mut Ui, color: &mut Color32, title: &str, opaque: bool) -> Response {
    let mut response = color_swatch_button(ui, *color, title);
    if follow_picker(ui, &response, color, |rgb, c| picked_color(rgb, *c, opaque)) {
        response.mark_changed();
    }
    if response.clicked() {
        stash_opening(ui, &response, color);
        let [r, g, b, _] = color.to_srgba_unmultiplied();
        crate::color_picker_ui::request_for_widget(ui.ctx(), response.id, title, [r, g, b].map(|v| f32::from(v) / 255.0));
    }
    response
}

fn picked_color(rgb: [f32; 3], original: Color32, opaque: bool) -> Color32 {
    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if !opaque && original.is_additive() {
        return Color32::from_rgb_additive(byte(rgb[0]), byte(rgb[1]), byte(rgb[2]));
    }
    Color32::from_rgba_unmultiplied(byte(rgb[0]), byte(rgb[1]), byte(rgb[2]), if opaque { 255 } else { original.to_srgba_unmultiplied()[3] })
}

/// [`color_edit_button_srgba`] for an opaque 8-bit sRGB colour.
pub fn color_edit_button_srgb(ui: &mut Ui, rgb: &mut [u8; 3], title: &str) -> Response {
    let mut color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
    let response = color_edit_button(ui, &mut color, title, true);
    if response.changed() {
        *rgb = [color.r(), color.g(), color.b()];
    }
    response
}

/// [`color_edit_button_srgba`] for a float colour stored as linear RGB (like egui's float RGB
/// widget). The Color Picker works in encoded sRGB; opening it and pressing OK unchanged keeps the
/// float value as it was.
pub fn color_edit_button_rgb(ui: &mut Ui, rgb: &mut [f32; 3], title: &str) -> Response {
    let mut response = color_swatch_button(ui, Color32::from(egui::Rgba::from_rgb(rgb[0], rgb[1], rgb[2])), title);
    if follow_picker(ui, &response, rgb, |color, _| color.map(egui::ecolor::linear_from_gamma)) {
        response.mark_changed();
    }
    if response.clicked() {
        stash_opening(ui, &response, rgb);
        let encoded = rgb.map(|c| if c.is_finite() { egui::ecolor::gamma_from_linear(c.clamp(0.0, 1.0)) } else { 0.0 });
        crate::color_picker_ui::request_for_widget(ui.ctx(), response.id, title, encoded);
    }
    response
}
