use super::fields::{FieldOpts, value_field_in};
use super::{Response, Ui};

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

/// A [`value_field_prec`] that keeps its trailing zeros (`50.0`, `0.0`): Warp's Bend, H and V,
/// which the reference shows to one decimal (#12503).
pub fn value_field_fixed(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, width: f32, decimals: u32) -> Response {
    value_field_in(ui, value, range, suffix, width, 0.0, FieldOpts { decimals: Some(decimals), keep_zeros: true, ..Default::default() }, None).0
}

/// A [`value_field_prec`] that shows an unset `0` as an empty field (Crop's free ratio sides,
/// #10888). Everything else, including scrubbing and typed input, is identical.
pub fn value_field_prec_blank(ui: &mut Ui, value: &mut f32, range: std::ops::RangeInclusive<f32>, suffix: &str, width: f32, decimals: u32) -> Response {
    value_field_in(ui, value, range, suffix, width, 0.0, FieldOpts { decimals: Some(decimals), blank_when_zero: true, ..Default::default() }, None).0
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

/// A [`value_field_extent`] showing the places [`value_field`](super::value_field) picks itself
/// (the Properties panel's Transform fields: `50%` typed into a px W is half of `extent`, #10784).
pub fn value_field_len(
    ui: &mut Ui,
    value: &mut f32,
    range: std::ops::RangeInclusive<f32>,
    suffix: &str,
    width: f32,
    units: FieldUnits,
    extent: f64,
) -> Response {
    let opts = FieldOpts { units: Some(units), extent: Some(extent), ..Default::default() };
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

/// [`fmt_num_prec`] keeping its trailing zeros: `50.0` at one place.
pub(super) fn fmt_num_fixed(v: f64, decimals: u32) -> String {
    let s = fmt_num_prec(v, decimals);
    let shown = s.split_once('.').map_or(0, |(_, frac)| frac.len());
    let pad = (decimals as usize).saturating_sub(shown);
    if pad == 0 { s } else { format!("{s}{}{}", if shown == 0 { "." } else { "" }, "0".repeat(pad)) }
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

/// The typed text without spaces (`1 234`), with a Unicode minus as `-`, and a decimal comma as a
/// point (`0,3`, as typed where the comma is the decimal separator, #10729) when the text has no
/// point; beside a point (`1.234,5`) a comma is left as it is, so the text stays invalid rather
/// than guessed.
///
/// The comma is read that way only where the system's separator is not the point
/// ([`crate::i18n::decimal_separator`]): where it is, a typed `1,234` is a thousands group, and
/// reading it as `1.234` would be out by a factor of a thousand rather than invalid (review on
/// #10729). Where the separator can't be read (macOS, Linux, the web) the field takes both, as
/// `zoom_field` does.
fn clean(text: &str) -> String {
    let bare: String = text.chars().filter(|c| !c.is_whitespace()).map(|c| if c == '−' { '-' } else { c }).collect();
    if crate::i18n::decimal_separator() == Some('.') {
        return bare;
    }
    // Each number decides for itself, so `1,5+2.0` commits 3.5 rather than being refused for a
    // point the other operand carries (CodeRabbit on #10729). A number holding both stays as it
    // is, so `1.234,5` is still invalid rather than guessed at.
    let mut out = String::with_capacity(bare.len());
    for part in bare.split_inclusive(|c: char| !matches!(c, '0'..='9' | ',' | '.')) {
        let (number, tail) = match part.chars().next_back().filter(|c| !matches!(c, '0'..='9' | ',' | '.')) {
            Some(end) => (&part[..part.len() - end.len_utf8()], Some(end)),
            None => (part, None),
        };
        if number.contains(',') && !number.contains('.') {
            out.extend(number.chars().map(|c| if c == ',' { '.' } else { c }));
        } else {
            out.push_str(number);
        }
        out.extend(tail);
    }
    out
}
