//! Descriptor adapters for the measured Stylize and Pixelate action events.

use super::descriptors::{desc, enm, px};
use super::fields::{Mapped, step};
use super::{D, Descriptor, Map, Value, json};

enum Kind {
    Long(i32, i32, i32),
    Pixels(f64, f64, f64),
    Bool(bool),
    Enum(&'static str, &'static [(&'static str, &'static str)], &'static str),
}

// Descriptor key, command parameter, and measured type/default/range.
struct Field(&'static str, &'static str, Kind);

const EXTRUDE: &[Field] = &[
    Field("ExtS", "size", Kind::Long(30, 2, 255)),
    Field("ExtD", "depth", Kind::Long(30, 1, 255)),
    Field("ExtF", "solidFrontFaces", Kind::Bool(false)),
    Field("ExtM", "maskIncompleteBlocks", Kind::Bool(false)),
    Field("ExtT", "type", Kind::Enum("ExtT", &[("Blks", "blocks"), ("Pyrm", "pyramids")], "blocks")),
    Field("ExtR", "depthMode", Kind::Enum("ExtR", &[("Rndm", "random"), ("LvlB", "levelBased")], "random")),
];
const TRACE: &[Field] = &[
    // Whole levels 0..255; the reference application's dialog clamps to the nearest.
    Field("Lvl ", "level", Kind::Long(128, 0, 255)),
    Field("Edg ", "edge", Kind::Enum("CntE", &[("Lwr ", "lower"), ("Upr ", "upper")], "upper")),
];
const WIND: &[Field] = &[
    Field("WndM", "method", Kind::Enum("WndM", &[("Wnd ", "wind"), ("Blst", "blast"), ("Stgr", "stagger")], "wind")),
    // Photoshop names the direction the streaks extend, rather than their source.
    Field("Drct", "direction", Kind::Enum("Drct", &[("Left", "fromRight"), ("Rght", "fromLeft")], "fromRight")),
];
const TILES: &[Field] = &[
    Field("TlNm", "count", Kind::Long(10, 1, 99)),
    Field("TlOf", "maxOffset", Kind::Long(10, 1, 99)),
    Field("FlCl", "fill", Kind::Enum("FlCl", &[("FlBc", "background"), ("FlFr", "foreground"), ("FlIn", "inverse"), ("FlSm", "unaltered")], "background")),
];
const HALFTONE: &[Field] = &[
    Field("Rds ", "maxRadius", Kind::Long(8, 4, 127)),
    Field("Ang1", "channel1", Kind::Long(108, -10000, 10000)),
    Field("Ang2", "channel2", Kind::Long(162, -10000, 10000)),
    Field("Ang3", "channel3", Kind::Long(90, -10000, 10000)),
    Field("Ang4", "channel4", Kind::Long(45, -10000, 10000)),
];
const MEZZOTINT: &[Field] = &[Field(
    "MztT",
    "type",
    Kind::Enum(
        "MztT",
        &[
            ("FnDt", "fineDots"),
            ("CrsD", "coarseDots"),
            ("GrnD", "grainyDots"),
            ("LngL", "longLines"),
            ("LngS", "longStrokes"),
            ("MdmD", "mediumDots"),
            ("MdmL", "mediumLines"),
            ("MdmS", "mediumStrokes"),
            ("ShSt", "shortStrokes"),
            ("ShrL", "shortLines"),
        ],
        "fineDots",
    ),
)];
const MOSAIC: &[Field] = &[Field("ClSz", "cellSize", Kind::Pixels(2.0, 2.0, 200.0))];
const POINTILLIZE: &[Field] = &[Field("ClSz", "cellSize", Kind::Long(5, 3, 300))];

/// The event's fields in recorded order; other filter families keep their own adapters.
fn fields(id: &str) -> Option<&'static [Field]> {
    Some(match id {
        "filter.stylize.extrude" => EXTRUDE,
        "filter.stylize.traceContour" => TRACE,
        "filter.stylize.wind" => WIND,
        "filter.stylize.tiles" => TILES,
        "filter.pixelate.colorHalftone" => HALFTONE,
        "filter.pixelate.mezzotint" => MEZZOTINT,
        "filter.pixelate.mosaic" => MOSAIC,
        "filter.pixelate.pointillize" => POINTILLIZE,
        "filter.stylize.findEdges" | "filter.stylize.solarize" | "filter.pixelate.facet" | "filter.pixelate.fragment" => &[],
        _ => return None,
    })
}

impl Kind {
    /// Missing ATN keys take the measured default, which can differ from the native command.
    fn default(&self) -> Value {
        match self {
            Self::Long(n, ..) => json!(n),
            Self::Pixels(n, ..) => json!(n),
            Self::Bool(b) => json!(b),
            Self::Enum(_, _, value) => json!(value),
        }
    }

    /// Reject incompatible types and enum identities; clamp only measured numeric ranges.
    fn read(&self, value: &D) -> Option<Value> {
        Some(match (self, value) {
            (Self::Long(_, min, max), D::Integer(n)) => json!(n.clamp(min, max)),
            (Self::Pixels(_, min, max), D::UnitFloat { unit, value: n }) if unit == b"#Pxl" && n.is_finite() => json!(n.clamp(*min, *max)),
            (Self::Pixels(_, min, max), D::Double(n)) if n.is_finite() => json!(n.clamp(*min, *max)),
            (Self::Pixels(_, min, max), D::Integer(n)) => json!(f64::from(*n).clamp(*min, *max)),
            (Self::Bool(_), D::Boolean(b)) => json!(b),
            (Self::Enum(ty, values, _), D::Enumerated { type_id, value }) if type_id.is(ty) => {
                json!(values.iter().find(|(code, _)| value.is(code))?.1)
            }
            _ => return None,
        })
    }

    /// Refuse native values the field cannot represent instead of changing the saved edit.
    fn write(&self, value: &Value) -> Option<D> {
        Some(match self {
            Self::Long(_, min, max) => {
                let n = value.as_f64()?;
                let integer = n as i32;
                if f64::from(integer) != n || !(*min..=*max).contains(&integer) {
                    return None;
                }
                D::Integer(integer)
            }
            Self::Pixels(_, min, max) => {
                let n = value.as_f64()?;
                if !n.is_finite() || !(*min..=*max).contains(&n) {
                    return None;
                }
                px(n)
            }
            Self::Bool(_) => D::Boolean(value.as_bool()?),
            Self::Enum(ty, values, _) => enm(ty, values.iter().find(|(_, label)| Some(*label) == value.as_str())?.0),
        })
    }
}

/// Unknown or duplicate keys keep the original step intact rather than lose settings.
fn params_from(id: &str, d: &Descriptor) -> Option<Map<String, Value>> {
    let fields = fields(id)?;
    if d.items.iter().any(|(key, _)| !fields.iter().any(|f| key.is(f.0))) {
        return None;
    }
    let mut params = Map::new();
    for Field(key, param, kind) in fields {
        if d.items.iter().filter(|(k, _)| k.is(key)).count() > 1 {
            return None;
        }
        let value = match d.get(key) {
            Some(value) => kind.read(value)?,
            None => kind.default(),
        };
        params.insert((*param).into(), value);
    }
    Some(params)
}

/// Map a complete measured descriptor or leave it as an unsupported Photoshop step.
pub(super) fn from(id: &str, d: &Descriptor) -> Mapped {
    match params_from(id, d) {
        Some(params) => step(id, Value::Object(params)),
        None => Mapped::Unsupported { harmless: false },
    }
}

/// Write every measured key, preserving native defaults and rejecting native-only settings.
pub(super) fn to(id: &str, p: &Value) -> Option<Descriptor> {
    let fields = fields(id)?;
    // No params mean the native defaults, as for the other mapped events.
    let empty = Map::new();
    let params = if p.is_null() { &empty } else { p.as_object()? };
    // The filter dialog freezes the current colours; the recorded event has none and plays with the current ones.
    let colours = matches!(id, "filter.stylize.tiles" | "filter.pixelate.pointillize");
    let seeded =
        matches!(id, "filter.stylize.extrude" | "filter.stylize.wind" | "filter.stylize.tiles" | "filter.pixelate.mezzotint" | "filter.pixelate.pointillize");
    if !super::events::pixel_target(p)
        || params.iter().any(|(key, value)| {
            !fields.iter().any(|f| f.1 == key.as_str())
                && key != "target"
                && !(colours && matches!(key.as_str(), "foreground" | "background"))
                && !(key == "seed" && seeded && (value.is_null() || value.as_f64() == Some(0.0)))
        })
    {
        return None;
    }
    let mut d = desc("null");
    for Field(key, param, kind) in fields {
        // Imports materialize Photoshop's defaults. Native commands retain their own defaults.
        let default = match (id, *param) {
            ("filter.pixelate.mosaic", "cellSize") => json!(10),
            ("filter.stylize.traceContour", "edge") => json!("lower"),
            _ => kind.default(),
        };
        let mut value = p.get(param).filter(|v| !v.is_null()).unwrap_or(&default).clone();
        // The native command rounds the tile count, so a fractional dialog value plays as the whole one.
        if (id, *param) == ("filter.stylize.tiles", "count")
            && let Some(n) = value.as_f64().filter(|n| n.is_finite())
        {
            value = json!(n.round());
        }
        d = d.with(key, kind.write(&value)?);
    }
    Some(d)
}
