use super::descriptors::{ang, color_desc, color_hex, enm, get_bool, get_enum, get_num, get_obj, long, pct, px};
use super::tables::{blend_label, blend_ps, lookup, lookup_back};
use super::*;

// ------------------------------------------------------------------ field tables

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Px,
    Pct,
    Ang,
    Long,
    /// An integer field whose value must survive conversion without rounding or saturation.
    ExactLong,
    Dbl,
    Bool,
    /// An enumeration: its type id and the Photoshop ↔ PhotoCraft value pairs.
    Enum(&'static str, &'static [(&'static str, &'static str)]),
    /// A blend mode (`BlnM`).
    Blend,
    /// A colour object ↔ `#rrggbb`.
    Color,
}

/// One descriptor key ↔ one command param.
pub(super) struct F(pub(super) &'static str, pub(super) &'static str, pub(super) Kind);

/// Import mapped descriptor fields, leaving absent values to command defaults.
/// Return `None` when a present enum, blend mode or colour cannot be represented.
pub(super) fn fields_from(d: &Descriptor, fields: &[F]) -> Option<Map<String, Value>> {
    let mut m = Map::new();
    for F(key, param, kind) in fields {
        let v = match kind {
            Kind::Px | Kind::Pct | Kind::Ang | Kind::Long | Kind::ExactLong | Kind::Dbl => get_num(d, key).map(Value::from),
            Kind::Bool => get_bool(d, key).map(Value::from),
            Kind::Enum(_, table) => match get_enum(d, key) {
                Some(e) => Some(Value::from(lookup(table, &e)?)),
                None => None,
            },
            Kind::Blend => match get_enum(d, key) {
                Some(e) => Some(Value::from(blend_label(&e)?)),
                None => None,
            },
            Kind::Color => match get_obj(d, key) {
                Some(c) => Some(Value::from(color_hex(c)?)),
                None => None,
            },
        };
        if let Some(v) = v {
            m.insert((*param).into(), v);
        }
    }
    Some(m)
}

/// Export supplied mapped params, omitting absent values to preserve event defaults.
/// Return `None` for unsupported values; `ExactLong` also rejects rounding or saturation.
pub(super) fn fields_to(p: &Value, fields: &[F], mut d: Descriptor) -> Option<Descriptor> {
    for F(key, param, kind) in fields {
        let Some(v) = p.get(param) else { continue };
        let out = match kind {
            Kind::Px => px(v.as_f64()?),
            Kind::Pct => pct(v.as_f64()?),
            Kind::Ang => ang(v.as_f64()?),
            // A value outside the descriptor's 32-bit integer refuses the step rather than being
            // clamped to another value (an Add Noise seed above i32::MAX replayed as i32::MAX, #7287).
            Kind::Long => {
                let n = v.as_f64()?.round();
                if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&n) {
                    return None;
                }
                long(n)
            }
            Kind::ExactLong => {
                let number = v.as_f64()?;
                let integer = number as i32;
                if f64::from(integer) != number {
                    return None;
                }
                D::Integer(integer)
            }
            Kind::Dbl => D::Double(v.as_f64()?),
            Kind::Bool => D::Boolean(v.as_bool()?),
            Kind::Enum(ty, table) => enm(ty, lookup_back(table, v.as_str()?)?),
            Kind::Blend => enm("BlnM", blend_ps(v.as_str()?)?),
            Kind::Color => D::Descriptor(color_desc(v.as_str()?)?),
        };
        d = d.with(key, out);
    }
    Some(d)
}

// ------------------------------------------------------------------ the event table

/// What one Photoshop step maps to.
pub(super) enum Mapped {
    /// One or more PhotoCraft steps (a `make` with a name becomes new layer + rename).
    Steps(Vec<(String, Value)>),
    /// No PhotoCraft equivalent; `harmless` steps (tool selections…) are imported switched off.
    Unsupported { harmless: bool },
}

pub(super) fn step(id: &str, params: Value) -> Mapped {
    Mapped::Steps(vec![(id.to_string(), params)])
}

pub(super) fn steps(v: Vec<(&str, Value)>) -> Mapped {
    Mapped::Steps(v.into_iter().map(|(id, p)| (id.to_string(), p)).collect())
}

pub(super) fn none(id: &str) -> Mapped {
    step(id, json!({}))
}
