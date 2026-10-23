//! Hue/Saturation smart filters (`Fltr` class `HStr`, filterID `HStr` as a long), as Photoshop
//! records Image › Adjustments › Hue/Saturation applied to a smart object:
//!
//! ```text
//! Fltr = HStr { presetKind = presetKindType.presetKindCustom, Clrz = bool,
//!   Adjs = [ Hst2 { H   , Strt, Lght }                                   (master, first)
//!            Hst2 { LclR (1 reds … 6 magentas), BgnR, BgnS, EndS, EndR,  (one per edited range)
//!                   H   , Strt, Lght } ] }
//! ```
//!
//! Every number is a long: hue in degrees (a shift of -180..180, or 0..360 with Colorize), the
//! rest in percent; the four range sliders are hue degrees (reds: 315, 345, 15, 45). The filter
//! maps to `image.adjustments.hueSaturation` with the command's own params, so it re-renders
//! when the smart object does. Anything else (an unknown key, a duplicated or unknown range, a
//! value Photoshop's dialog can't hold) stays an opaque [`super::UNSUPPORTED_FILTER`].

use photocraft_doc::SmartFilter;
use photocraft_doc::adjust::HueRange;
use photocraft_psd::descriptor::{Descriptor, UnicodeString, Value};
use serde_json::{Map, Value as J, json};

use super::{blend_options, enumv, is_meta, rgbc};
use crate::blocks::num;

pub(super) const CLASS: &str = "HStr";
pub(super) const COMMAND: &str = "image.adjustments.hueSaturation";
const FILTER_ID: i32 = super::code(b"HStr");
/// `Nm  ` (Photoshop writes the localized menu item).
const NAME: &str = "Hue/Saturation...";
/// The `Fltr` descriptor's own name.
const FLTR_NAME: &str = "Hue/Saturation";

/// The colour ranges in `LclR` order (1 = reds), as the command's param keys.
const RANGES: [&str; 6] = ["reds", "yellows", "greens", "cyans", "blues", "magentas"];
const BOUNDS: [&str; 4] = ["BgnR", "BgnS", "EndS", "EndR"];
const SETTINGS: [(&str, &str); 3] = [("H   ", "hue"), ("Strt", "saturation"), ("Lght", "lightness")];

/// A whole, finite number within `lo..=hi`, as JSON.
fn whole(v: Option<&Value>, lo: f64, hi: f64) -> Option<J> {
    let v = num(v).filter(|v| v.is_finite() && v.fract() == 0.0 && (lo..=hi).contains(v))?;
    Some(json!(v as i64))
}

/// `image.adjustments.hueSaturation` params for an `HStr` filter; `None` when any part of it is
/// not the observed layout.
pub(super) fn import_params(f: &Descriptor) -> Option<J> {
    if !f.class_id.is(CLASS) || f.items.iter().any(|(k, _)| !["presetKind", "Clrz", "Adjs"].iter().any(|n| k.is(n))) {
        return None;
    }
    let colorize = match f.get("Clrz") {
        Some(Value::Boolean(b)) => *b,
        None => false,
        Some(_) => return None,
    };
    let Some(Value::List(entries)) = f.get("Adjs") else { return None };
    // A master and six ranges at most.
    if entries.len() > 1 + RANGES.len() {
        return None;
    }
    let mut out = Map::new();
    out.insert("colorize".into(), J::Bool(colorize));
    let mut seen_master = false;
    for e in entries {
        let Value::Descriptor(e) = e else { return None };
        if e.items.iter().any(|(k, _)| !k.is("LclR") && !BOUNDS.iter().any(|n| k.is(n)) && !SETTINGS.iter().any(|(n, _)| k.is(n))) {
            return None;
        }
        let mut o = Map::new();
        match e.get("LclR") {
            None => {
                if std::mem::replace(&mut seen_master, true) {
                    return None;
                }
                let (hue, sat) = if colorize { ((0.0, 360.0), (0.0, 100.0)) } else { ((-180.0, 180.0), (-100.0, 100.0)) };
                o.insert("hue".into(), whole(e.get("H   "), hue.0, hue.1)?);
                o.insert("saturation".into(), whole(e.get("Strt"), sat.0, sat.1)?);
                o.insert("lightness".into(), whole(e.get("Lght"), -100.0, 100.0)?);
                out.extend(o);
            }
            r => {
                let i = whole(r, 1.0, RANGES.len() as f64)?.as_u64()?;
                let key = *RANGES.get(usize::try_from(i).ok()?.checked_sub(1)?)?;
                if out.contains_key(key) {
                    return None;
                }
                o.insert("hue".into(), whole(e.get("H   "), -180.0, 180.0)?);
                o.insert("saturation".into(), whole(e.get("Strt"), -100.0, 100.0)?);
                o.insert("lightness".into(), whole(e.get("Lght"), -100.0, 100.0)?);
                let bounds: Option<Vec<J>> = BOUNDS.iter().map(|k| whole(e.get(k), -720.0, 720.0)).collect();
                o.insert("range".into(), J::Array(bounds?));
                out.insert(key.into(), J::Object(o));
            }
        }
    }
    Some(J::Object(out))
}

/// A number param of `o` (`default` when absent), finite.
fn number(o: &J, key: &str, default: f64) -> Result<f64, String> {
    match o.get(key) {
        None => Ok(default),
        Some(v) => v.as_f64().filter(|v| v.is_finite()).ok_or(format!("Hue/Saturation `{key}` must be a number")),
    }
}

/// A number param rounded and clamped like the dialog's field, as a long.
fn setting(o: &J, key: &str, default: f64, lo: f64, hi: f64) -> Result<Value, String> {
    Ok(long(number(o, key, default)?, lo, hi))
}

fn long(v: f64, lo: f64, hi: f64) -> Value {
    Value::Integer(v.round().clamp(lo, hi) as i32)
}

/// The `Adjs` entry of colour range `i` (0 = reds), or `None` when it is neutral with its default
/// extent (Photoshop lists only the ranges that were edited).
fn range_entry(i: usize, o: &J) -> Result<Option<Value>, String> {
    let obj = o.as_object().ok_or(format!("Hue/Saturation `{}` must be an object", RANGES.get(i).unwrap_or(&"")))?;
    if let Some(k) = obj.keys().find(|k| !["hue", "saturation", "lightness", "range"].contains(&k.as_str())) {
        return Err(format!("Hue/Saturation range setting `{k}` has no PSD equivalent"));
    }
    let neutral = HueRange::neutral(i);
    let bounds = match o.get("range") {
        None => neutral.bounds,
        Some(r) => {
            let a = r.as_array().filter(|a| a.len() == 4).ok_or("a Hue/Saturation range must be four hue degrees")?;
            let mut b = [0f32; 4];
            for (slot, v) in b.iter_mut().zip(a) {
                *slot = v.as_f64().filter(|v| v.is_finite()).ok_or("a Hue/Saturation range must be four hue degrees")? as f32;
            }
            HueRange::canonical_bounds(b)
        }
    };
    let hue = setting(o, "hue", 0.0, -180.0, 180.0)?;
    let sat = setting(o, "saturation", 0.0, -100.0, 100.0)?;
    let light = setting(o, "lightness", 0.0, -100.0, 100.0)?;
    let zero = Value::Integer(0);
    if [&hue, &sat, &light].iter().all(|v| **v == zero) && bounds == neutral.bounds {
        return Ok(None);
    }
    let lclr = i32::try_from(i + 1).map_err(|e| e.to_string())?;
    let mut d = Descriptor::new("Hst2").with("LclR", Value::Integer(lclr));
    for (k, v) in BOUNDS.iter().zip(bounds) {
        // Photoshop writes the sliders within one turn (reds: 315, 345, 15, 45).
        d = d.with(k, Value::Integer(f64::from(v).round().rem_euclid(360.0) as i32));
    }
    Ok(Some(Value::Descriptor(d.with("H   ", hue).with("Strt", sat).with("Lght", light))))
}

/// The `filterFXList` item for an `image.adjustments.hueSaturation` smart filter.
pub(super) fn export_item(f: &SmartFilter) -> Result<Descriptor, String> {
    let p = &f.params;
    let obj = match p {
        J::Object(o) => o,
        J::Null => &Map::new(),
        _ => return Err("Hue/Saturation parameters must be an object".into()),
    };
    let known = |k: &str| ["hue", "saturation", "lightness", "colorize"].contains(&k) || RANGES.contains(&k);
    if let Some(k) = obj.keys().find(|k| !is_meta(k) && !known(k)) {
        return Err(format!("Hue/Saturation setting `{k}` has no PSD equivalent"));
    }
    let colorize = match p.get("colorize") {
        None => false,
        Some(v) => v.as_bool().ok_or("Hue/Saturation `colorize` must be true or false")?,
    };
    // As the command: Colorize hue is 0..360 and starts at 25 % saturation unless one is given.
    let master = if colorize {
        (long(number(p, "hue", 0.0)?.rem_euclid(360.0), 0.0, 360.0), setting(p, "saturation", 25.0, 0.0, 100.0)?)
    } else {
        (setting(p, "hue", 0.0, -180.0, 180.0)?, setting(p, "saturation", 0.0, -100.0, 100.0)?)
    };
    let mut adjs = vec![Value::Descriptor(
        Descriptor::new("Hst2").with("H   ", master.0).with("Strt", master.1).with("Lght", setting(p, "lightness", 0.0, -100.0, 100.0)?),
    )];
    for (i, key) in RANGES.iter().enumerate() {
        if let Some(o) = p.get(*key) {
            adjs.extend(range_entry(i, o)?);
        }
    }
    let mut fltr = Descriptor::new(CLASS)
        .with("presetKind", enumv("presetKindType", "presetKindCustom"))
        .with("Clrz", Value::Boolean(colorize))
        .with("Adjs", Value::List(adjs));
    fltr.name = UnicodeString::new_nul(FLTR_NAME);
    Ok(Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul(NAME)))
        .with("blendOptions", blend_options(f))
        .with("enab", Value::Boolean(f.visible))
        .with("hasoptions", Value::Boolean(true))
        .with("FrgC", rgbc(0.0))
        .with("BckC", rgbc(255.0))
        .with("Fltr", Value::Descriptor(fltr))
        .with("filterID", Value::Integer(FILTER_ID)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smart_map::{UNSUPPORTED_FILTER, filter_from_item, item_for_filter};
    use photocraft_color::BlendMode;
    use photocraft_psd::descriptor::Id;

    fn hst2(range: Option<(i32, [i32; 4])>, h: i32, s: i32, l: i32) -> Value {
        let mut d = Descriptor::new("Hst2");
        if let Some((r, b)) = range {
            d = d.with("LclR", Value::Integer(r));
            for (k, v) in BOUNDS.iter().zip(b) {
                d = d.with(k, Value::Integer(v));
            }
        }
        Value::Descriptor(d.with("H   ", Value::Integer(h)).with("Strt", Value::Integer(s)).with("Lght", Value::Integer(l)))
    }

    fn entry(items: &[(&str, Value)]) -> Value {
        Value::Descriptor(items.iter().fold(Descriptor::new("Hst2"), |d, (k, v)| d.with(k, v.clone())))
    }

    fn fltr(colorize: bool, adjs: Vec<Value>) -> Descriptor {
        let mut f = Descriptor::new(CLASS)
            .with("presetKind", enumv("presetKindType", "presetKindCustom"))
            .with("Clrz", Value::Boolean(colorize))
            .with("Adjs", Value::List(adjs));
        f.name = UnicodeString::new_nul("Farbton/Sättigung");
        f
    }

    /// A `filterFXList` item exactly as Photoshop saved it (German UI: localized names, the
    /// foreground colour of the moment in `FrgC`).
    fn item(f: Descriptor) -> Descriptor {
        let c = |r: f64, g: f64, b: f64| {
            Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(r)).with("Grn ", Value::Double(g)).with("Bl  ", Value::Double(b)))
        };
        let neutral = SmartFilter { command: String::new(), params: J::Null, blend: BlendMode::Normal, opacity: 1.0, visible: true };
        Descriptor::new("filterFX")
            .with("Nm  ", Value::Text(UnicodeString::new_nul("Farbton/Sättigung...")))
            .with("blendOptions", blend_options(&neutral))
            .with("enab", Value::Boolean(true))
            .with("hasoptions", Value::Boolean(true))
            .with("FrgC", c(167.0, 157.0, 147.0))
            .with("BckC", c(255.0, 255.0, 255.0))
            .with("Fltr", Value::Descriptor(f))
            .with("filterID", Value::Integer(FILTER_ID))
    }

    /// Master 12 / -20 / 5 and Yellows -16 / 10 / -5 over 15, 45, 75, 105, Colorize off.
    fn observed() -> Descriptor {
        item(fltr(false, vec![hst2(None, 12, -20, 5), hst2(Some((2, [15, 45, 75, 105])), -16, 10, -5)]))
    }

    fn fltr_of(d: &Descriptor) -> &Descriptor {
        match d.get("Fltr") {
            Some(Value::Descriptor(f)) => f,
            _ => panic!("no Fltr"),
        }
    }

    fn filter(params: J) -> SmartFilter {
        SmartFilter { command: COMMAND.into(), params, blend: BlendMode::Normal, opacity: 1.0, visible: true }
    }

    #[test]
    fn filter_id_is_hstr() {
        assert_eq!(FILTER_ID, 1213428850);
    }

    #[test]
    fn photoshop_items_map_to_the_command() {
        let f = filter_from_item(&observed());
        assert_eq!(f.command, COMMAND);
        assert_eq!(
            f.params,
            json!({"colorize": false, "hue": 12, "saturation": -20, "lightness": 5,
                   "yellows": {"hue": -16, "saturation": 10, "lightness": -5, "range": [15, 45, 75, 105]}})
        );
        let c = filter_from_item(&item(fltr(true, vec![hst2(None, 200, 40, -10)])));
        assert_eq!(c.command, COMMAND);
        assert_eq!(c.params, json!({"colorize": true, "hue": 200, "saturation": 40, "lightness": -10}));
    }

    #[test]
    fn export_writes_photoshops_layout_and_round_trips() {
        let reds = item(fltr(false, vec![hst2(None, 0, 0, 0), hst2(Some((1, [315, 345, 15, 45])), 30, 0, 0)]));
        for src in [observed(), item(fltr(true, vec![hst2(None, 200, 40, -10)])), reds] {
            let f = filter_from_item(&src);
            let out = item_for_filter(&f).unwrap();
            let keys = |d: &Descriptor| d.items.iter().map(|(k, _)| k.clone()).collect::<Vec<Id>>();
            assert_eq!(keys(&out), keys(&src), "item key order");
            let (a, b) = (fltr_of(&out), fltr_of(&src));
            assert_eq!((&a.class_id, &a.items), (&b.class_id, &b.items), "Fltr as Photoshop writes it");
            assert_eq!(out.get("filterID"), Some(&Value::Integer(FILTER_ID)));
            assert_eq!(filter_from_item(&out), f);
        }
        // Blending and visibility are the model's own.
        let mut f = filter_from_item(&observed());
        (f.opacity, f.blend, f.visible) = (0.5, BlendMode::Screen, false);
        assert_eq!(filter_from_item(&item_for_filter(&f).unwrap()), f);
    }

    #[test]
    fn command_params_are_written_like_the_dialog() {
        let export = |p: J| filter_from_item(&item_for_filter(&filter(p)).unwrap()).params;
        // Partial params: the command's defaults, numbers rounded and clamped.
        assert_eq!(export(json!({"hue": 12.4})), json!({"colorize": false, "hue": 12, "saturation": 0, "lightness": 0}));
        assert_eq!(export(json!({"hue": -400, "lightness": 101, "layer": 3})), json!({"colorize": false, "hue": -180, "saturation": 0, "lightness": 100}));
        assert_eq!(export(J::Null), json!({"colorize": false, "hue": 0, "saturation": 0, "lightness": 0}));
        // Colorize starts at 25 % saturation and wraps the hue into 0..360.
        assert_eq!(export(json!({"colorize": true, "hue": -30})), json!({"colorize": true, "hue": 330, "saturation": 25, "lightness": 0}));
        // The full set (`to_params`): neutral ranges at their default extent are left out, reds
        // are written within one turn.
        let mut full = json!({"hue": 0, "saturation": 0, "lightness": 0, "colorize": false});
        for (i, k) in RANGES.iter().enumerate() {
            let r = HueRange::neutral(i);
            full[*k] = json!({"hue": r.hue, "saturation": r.saturation, "lightness": r.lightness, "range": r.bounds});
        }
        full["reds"]["saturation"] = json!(-50.0);
        full["blues"]["range"] = json!([190, 220, 250, 280]);
        let back = export(full);
        assert_eq!(back["reds"], json!({"hue": 0, "saturation": -50, "lightness": 0, "range": [315, 345, 15, 45]}));
        assert_eq!(back["blues"], json!({"hue": 0, "saturation": 0, "lightness": 0, "range": [190, 220, 250, 280]}));
        assert_eq!(RANGES.iter().filter(|k| back.get(**k).is_some()).count(), 2, "{back}");
    }

    #[test]
    fn unreadable_items_stay_opaque_and_are_written_back() {
        let zero = Value::Integer(0);
        let b4 = [0, 30, 60, 90];
        let cases = vec![
            fltr(false, vec![hst2(None, 1, 2, 3)]).with("Lab ", Value::Boolean(true)),
            fltr(false, vec![hst2(None, 1, 2, 3), hst2(None, 1, 2, 3)]),
            fltr(false, vec![hst2(None, 0, 0, 0), hst2(Some((2, b4)), 1, 0, 0), hst2(Some((2, b4)), 2, 0, 0)]),
            fltr(false, vec![hst2(Some((0, b4)), 1, 0, 0)]),
            fltr(false, vec![hst2(Some((7, b4)), 1, 0, 0)]),
            fltr(false, vec![hst2(Some((-1, b4)), 1, 0, 0)]),
            fltr(false, vec![hst2(None, 181, 0, 0)]),
            fltr(false, vec![hst2(None, 0, -101, 0)]),
            fltr(true, vec![hst2(None, -1, 0, 0)]),
            fltr(true, vec![hst2(None, 10, -5, 0)]),
            fltr(false, vec![hst2(Some((3, [0, 1, 2, 9999])), 1, 0, 0)]),
            fltr(false, vec![Value::Integer(4)]),
            fltr(false, vec![entry(&[("H   ", zero.clone()), ("Strt", zero.clone()), ("Lght", zero.clone()), ("Clr ", zero.clone())])]),
            fltr(false, vec![entry(&[("H   ", Value::Double(f64::NAN)), ("Strt", zero.clone()), ("Lght", zero.clone())])]),
            fltr(false, vec![entry(&[("H   ", Value::Double(1.5)), ("Strt", zero.clone()), ("Lght", zero.clone())])]),
            fltr(false, vec![entry(&[("H   ", Value::Boolean(true)), ("Strt", zero.clone()), ("Lght", zero.clone())])]),
            fltr(false, vec![entry(&[("Strt", zero.clone()), ("Lght", zero.clone())])]),
            fltr(false, vec![entry(&[("LclR", Value::Integer(1)), ("H   ", zero.clone()), ("Strt", zero.clone()), ("Lght", zero.clone())])]),
            fltr(false, (0..8).map(|i| hst2(Some((i % 6 + 1, b4)), 0, 0, 0)).collect()),
            Descriptor::new(CLASS).with("Clrz", Value::Integer(1)).with("Adjs", Value::List(vec![hst2(None, 1, 2, 3)])),
            Descriptor::new(CLASS).with("Clrz", Value::Boolean(false)),
            Descriptor::new(CLASS).with("Adjs", Value::Integer(0)),
        ];
        for (n, f) in cases.into_iter().enumerate() {
            let src = item(f);
            let sf = filter_from_item(&src);
            assert_eq!(sf.command, UNSUPPORTED_FILTER, "case {n}");
            assert_eq!(item_for_filter(&sf).unwrap().to_bytes(), src.to_bytes(), "case {n}");
        }
        // No `Fltr` at all.
        let mut bare = observed();
        bare.items.retain(|(k, _)| !k.is("Fltr"));
        assert_eq!(filter_from_item(&bare).command, UNSUPPORTED_FILTER);
    }

    #[test]
    fn bad_exports_report_errors() {
        for p in [
            json!(3),
            json!({"hue": "red"}),
            json!({"colorize": "yes"}),
            json!({"preset": "sepia"}),
            json!({"reds": 4}),
            json!({"reds": {"hue": "x"}}),
            json!({"reds": {"range": [1, 2, 3]}}),
            json!({"reds": {"range": [1, 2, 3, "x"]}}),
            json!({"reds": {"width": 2}}),
        ] {
            assert!(item_for_filter(&filter(p.clone())).is_err(), "{p}");
        }
        // Huge numbers clamp rather than overflow.
        let out = filter_from_item(&item_for_filter(&filter(json!({"hue": f64::MAX, "colorize": true, "saturation": -1e300}))).unwrap());
        assert_eq!(out.command, COMMAND);
    }
}
