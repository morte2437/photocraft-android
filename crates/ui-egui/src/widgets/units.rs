use super::fields::{FieldOpts, value_field_in};
use super::*;

/// A size dialog's unit key (`px`, `in`, `cm`, `mm`, `pt`, `pica`, `percent`) as a unit.
pub fn unit_of_key(key: &str) -> photocraft_engine::prefs::Unit {
    use photocraft_engine::prefs::Unit;
    match key {
        "in" => Unit::Inches,
        "cm" => Unit::Centimeters,
        "mm" => Unit::Millimeters,
        "pt" => Unit::Points,
        "pica" => Unit::Picas,
        "percent" => Unit::Percent,
        _ => Unit::Pixels,
    }
}

/// Decimal places for a size dialog's unit key (`px`, `in`, `cm`, `mm`, `pt`, `pica`, `percent`).
/// The places themselves live on [`photocraft_engine::prefs::Unit`], so the dialogs, the Info
/// panel and the rulers agree (#2434).
pub fn unit_decimals(key: &str) -> u32 {
    unit_of_key(key).decimals() as u32
}

/// A [`value_field`] showing `decimals` decimal places instead of the one (or two, for a small
/// range) it picks itself: the ruler units need their own precision (#2434).
pub fn value_field_prec(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, width: f32, decimals: u32) -> Response {
    value_field_in(ui, value, range, suffix, width, 0.0, FieldOpts { decimals: Some(decimals), ..Default::default() }, None).0
}

/// A field's unit and the resolution a typed unit converts at (`1in` in a px field).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldUnits {
    pub unit: photocraft_engine::prefs::Unit,
    /// Pixels per inch.
    pub dpi: f64,
    /// Points per inch (72 PostScript, 72.27 traditional).
    pub ppi: f64,
}

/// A [`value_field_prec`] without a suffix, whose unit is chosen beside it (Image Size and New
/// Document's unit menus) and converts typed units at that dialog's own resolution.
pub fn value_field_units(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32, decimals: u32, units: FieldUnits) -> Response {
    value_field_in(ui, value, range, "", width, 0.0, FieldOpts { decimals: Some(decimals), units: Some(units), ..Default::default() }, None).0
}

/// A [`value_field_prec`] in `units` whose 100 % is a length of `extent` px (Free Transform's W
/// and H): a length typed while it shows percent becomes that share of `extent` (`40px`, `2in`),
/// and a percentage typed while it shows a length becomes that share of `extent` (`50%`), at
/// `units`' resolution (#6284). The field keeps its own unit.
#[allow(clippy::too_many_arguments)]
pub fn value_field_extent(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    width: f32,
    decimals: u32,
    units: FieldUnits,
    extent: f64,
) -> Response {
    let opts = FieldOpts { decimals: Some(decimals), units: Some(units), extent: Some(extent), ..Default::default() };
    value_field_in(ui, value, range, suffix, width, 0.0, opts, None).0
}

pub(super) fn field_resolution_id() -> egui::Id {
    egui::Id::unique("pc-field-resolution")
}

/// The resolution (pixels per inch) and point size (points per inch) a unit typed into a value
/// field converts at: the active document's. The app sets it each frame; 72 / 72 without it.
pub fn set_field_resolution(ctx: &egui::Context, dpi: f64, ppi: f64) {
    ctx.data_mut(|d| d.insert_temp(field_resolution_id(), (dpi, ppi)));
}

/// The unit a field's suffix names (`px`, `in`, `cm`, `mm`, `pt`, `pica`/`pc`, `%`), if any.
pub(super) fn unit_of_suffix(suffix: &str) -> Option<photocraft_engine::prefs::Unit> {
    use photocraft_engine::prefs::Unit;
    Some(match suffix {
        "px" => Unit::Pixels,
        "in" => Unit::Inches,
        "cm" => Unit::Centimeters,
        "mm" => Unit::Millimeters,
        "pt" => Unit::Points,
        "pica" | "pc" => Unit::Picas,
        "%" => Unit::Percent,
        _ => return None,
    })
}

/// Photoshop-style numbers: "100", "12.5" (never "100.0").
pub fn fmt_num(v: f64) -> String {
    fmt_num_prec(v, 1)
}

/// Rounds `v` to `decimals` places (half away from zero) and drops trailing zeros, so a value is
/// never shown with more precision than it has: `0.7995` at three places is `0.8`, `2.0` is `2`
/// (#2434). [`photocraft_engine::prefs::Unit::decimals`] gives each ruler unit's places.
pub fn fmt_num_prec(v: f64, decimals: u32) -> String {
    photocraft_engine::prefs::fmt_decimals(v, decimals as usize)
}

/// Two-decimal variant of [`fmt_num`] for small ranges: `1.05`, `0.78`, `2`.
pub fn fmt_num2(v: f64) -> String {
    fmt_num_prec(v, 2)
}

/// Parses a typed number or bounded arithmetic, including parentheses and powers.
/// Preserves egui-compatible whitespace and Unicode minus handling.
pub fn parse_num(text: &str) -> Option<f64> {
    if text.len() > 1024 {
        return None;
    }
    crate::numeric_expression::parse(&clean(text))
}

/// [`parse_num`] in a field with `units`: a number may carry a unit (`2in`, `5mm`, `50%`),
/// converted into the field's unit; one that can't convert makes the text invalid.
pub(crate) fn parse_num_in(text: &str, units: crate::numeric_expression::Units) -> Option<f64> {
    if text.len() > 1024 {
        return None;
    }
    crate::numeric_expression::parse_with_units(&clean(text), Some(units))
}

/// A plain typed number, no arithmetic.
pub(super) fn plain(text: &str) -> Option<f64> {
    clean(text).parse().ok()
}

fn clean(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).map(|c| if c == '−' { '-' } else { c }).collect()
}
