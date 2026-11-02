//! Validate new filter invocations, without changing how old smart-filter records decode.

use serde_json::Value;

use crate::{EngineError, Result};

#[derive(Clone, Copy)]
enum Rule {
    Number(f64, f64),
    Unsigned(u64),
    Choice(&'static str),
    Bool,
}

impl Rule {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Number(min, max) => value.as_f64().is_some_and(|n| n.is_finite() && (min..=max).contains(&n)),
            Self::Unsigned(max) => value.as_u64().is_some_and(|n| n <= max),
            Self::Choice(choices) => value.as_str().is_some_and(|s| choices.split('|').any(|choice| choice == s)),
            Self::Bool => value.is_boolean(),
        }
    }

    fn expected(self) -> String {
        match self {
            Self::Number(min, max) if min == f64::MIN && max == f64::MAX => "a finite number".into(),
            Self::Number(min, max) => format!("a number in {min}..{max}"),
            Self::Unsigned(max) => format!("an integer in 0..{max}"),
            Self::Choice(choices) => format!("one of {choices}"),
            Self::Bool => "a boolean".into(),
        }
    }
}

// The bounds and choices are the filters' public dialog-unit contract. Defaults remain in
// params_for; gallery filters are checked in gallery_cmds.
fn fields(id: &str) -> Option<&'static [(&'static str, Rule)]> {
    use Rule::{Bool, Choice, Number, Unsigned};
    Some(match id {
        "filter.blur.gaussianBlur" | "filter.other.highPass" => &[("radius", Number(0.1, 1000.0))],
        "filter.blur.boxBlur" => &[("radius", Number(1.0, 2000.0))],
        "filter.blur.motionBlur" => &[("angle", Number(-360.0, 360.0)), ("distance", Number(1.0, 2000.0))],
        "filter.blur.radialBlur" => &[
            ("amount", Number(1.0, 100.0)),
            ("method", Choice("spin|zoom")),
            ("quality", Choice("draft|good|best")),
            ("centerX", Number(0.0, 1.0)),
            ("centerY", Number(0.0, 1.0)),
        ],
        "filter.blur.smartBlur" => &[
            ("radius", Number(0.1, 100.0)),
            ("threshold", Number(0.1, 100.0)),
            ("quality", Choice("high|medium|low")),
            ("mode", Choice("normal|edgeOnly|overlayEdge")),
        ],
        "filter.blur.shapeBlur" => &[("radius", Number(5.0, 1000.0)), ("shape", Choice("circle|ring|square|diamond|triangle|hexagon|star|heart|cross"))],
        "filter.noise.reduceNoise" => &[
            ("strength", Number(0.0, 10.0)),
            ("preserveDetails", Number(0.0, 100.0)),
            ("reduceColorNoise", Number(0.0, 100.0)),
            ("sharpenDetails", Number(0.0, 100.0)),
            ("removeJpegArtifact", Bool),
        ],
        "filter.blur.surfaceBlur" => &[("radius", Number(1.0, 100.0)), ("threshold", Number(2.0, 255.0))],
        "filter.sharpen.unsharpMask" => &[("amount", Number(1.0, 500.0)), ("radius", Number(0.1, 1000.0)), ("threshold", Number(0.0, 255.0))],
        "filter.sharpen.smartSharpen" => &[("amount", Number(1.0, 500.0)), ("radius", Number(0.1, 64.0)), ("reduceNoise", Number(0.0, 100.0))],
        "filter.noise.addNoise" => {
            &[("amount", Number(0.1, 400.0)), ("distribution", Choice("uniform|gaussian")), ("monochromatic", Bool), ("seed", Unsigned(u32::MAX as u64))]
        }
        "filter.noise.median" => &[("radius", Number(1.0, 500.0))],
        "filter.noise.dustAndScratches" => &[("radius", Number(1.0, 500.0)), ("threshold", Number(0.0, 255.0))],
        "filter.pixelate.mosaic" => &[("cellSize", Number(2.0, 200.0))],
        "filter.stylize.emboss" => &[("angle", Number(-180.0, 180.0)), ("height", Number(1.0, 100.0)), ("amount", Number(1.0, 500.0))],
        "filter.distort.twirl" => &[("angle", Number(-999.0, 999.0))],
        "filter.distort.pinch" => &[("amount", Number(-100.0, 100.0))],
        "filter.distort.spherize" => &[("amount", Number(-100.0, 100.0)), ("mode", Choice("normal|horizontalOnly|verticalOnly"))],
        "filter.distort.wave" => &[
            ("generators", Number(1.0, 999.0)),
            ("wavelengthMin", Number(1.0, 998.0)),
            ("wavelengthMax", Number(2.0, 999.0)),
            ("amplitudeMin", Number(1.0, 998.0)),
            ("amplitudeMax", Number(1.0, 999.0)),
            ("type", Choice("sine|triangle|square")),
            ("undefinedAreas", Choice("wrap|repeat")),
            ("seed", Unsigned(u32::MAX as u64)),
        ],
        "filter.distort.ripple" => &[("amount", Number(-999.0, 999.0)), ("size", Choice("small|medium|large"))],
        "filter.distort.polarCoordinates" => &[("mode", Choice("rectangularToPolar|polarToRectangular"))],
        "filter.other.minimum" | "filter.other.maximum" => &[("radius", Number(0.2, 500.0)), ("preserve", Choice("squareness|roundness"))],
        "filter.other.offset" => &[
            // The hint specifies pixels, with no range: keep the existing rounding/saturation.
            ("horizontal", Number(f64::MIN, f64::MAX)),
            ("vertical", Number(f64::MIN, f64::MAX)),
            ("undefinedAreas", Choice("wrap|repeat|transparent")),
        ],
        "filter.blur.blur"
        | "filter.blur.blurMore"
        | "filter.sharpen.sharpen"
        | "filter.sharpen.sharpenMore"
        | "filter.sharpen.sharpenEdges"
        | "filter.noise.despeckle"
        | "filter.stylize.findEdges"
        | "filter.stylize.solarize" => &[],
        _ => return None,
    })
}

// Remaining extended filters: only their documented choices are checked for now, since some
// accept extra keys (colours, map documents) and their numeric ranges clamp in params_for.
fn choices(id: &str) -> Option<&'static [(&'static str, &'static str)]> {
    Some(match id {
        "filter.pixelate.mezzotint" => {
            &[("type", "fineDots|mediumDots|grainyDots|coarseDots|shortLines|mediumLines|longLines|shortStrokes|mediumStrokes|longStrokes")]
        }
        "filter.stylize.diffuse" => &[("mode", "normal|darkenOnly|lightenOnly|anisotropic")],
        "filter.stylize.extrude" => &[("type", "blocks|pyramids"), ("depthMode", "random|levelBased")],
        "filter.stylize.tiles" => &[("fill", "background|foreground|inverse|unaltered")],
        "filter.stylize.traceContour" => &[("edge", "lower|upper")],
        "filter.stylize.wind" => &[("method", "wind|blast|stagger"), ("direction", "fromRight|fromLeft")],
        "filter.distort.displace" => &[("fit", "stretch|tile"), ("undefinedAreas", "repeat|wrap")],
        "filter.distort.shear" => &[("undefinedAreas", "wrap|repeat")],
        "filter.distort.zigZag" => &[("style", "pondRipples|outFromCenter|aroundCenter")],
        "filter.render.lensFlare" => &[("lens", "zoom|prime35|prime105|moviePrime")],
        "filter.render.lightingEffects" => &[("lightType", "spot|point|infinite"), ("texture", "none|red|green|blue|alpha|luminance")],
        "filter.blur.lensBlur" => &[
            ("shape", "hexagon|triangle|square|pentagon|heptagon|octagon"),
            ("depthMap", "none|transparency|layerMask|channel"),
            ("distribution", "uniform|gaussian"),
        ],
        "filter.other.hsbHsl" => &[("inputMode", "rgb|hsb|hsl"), ("rowOrder", "hsb|hsl|rgb")],
        "filter.video.deInterlace" => &[("eliminate", "oddFields|evenFields"), ("createBy", "interpolation|duplication")],
        _ => return None,
    })
}

/// Runs both at dispatch (before committing a floating selection) and at the shared runner
/// (direct specs/Last Filter). Filters outside the core and extended families are unaffected.
pub(crate) fn validate_params(id: &str, params: &Value) -> Result<()> {
    let bad = |msg| EngineError::BadParams { cmd: id.into(), msg };
    if let Some(choices) = choices(id) {
        for &(key, allowed) in choices {
            if let Some(value) = params.get(key)
                && !Rule::Choice(allowed).accepts(value)
            {
                return Err(bad(format!("`{key}` must be {}", Rule::Choice(allowed).expected())));
            }
        }
        return Ok(());
    }
    let Some(fields) = fields(id) else { return crate::gallery_cmds::validate_params(id, params) };
    if params.is_null() {
        return Ok(());
    }
    let params = params.as_object().ok_or_else(|| bad("params must be a JSON object".into()))?;
    for (key, value) in params {
        let (valid, expected) = if let Some((_, rule)) = fields.iter().find(|(name, _)| *name == key) {
            (rule.accepts(value), rule.expected())
        } else {
            match key.as_str() {
                "layer" => (value.as_u64().is_some() || value.is_string(), "an unsigned layer id or layer name".into()),
                "coalesce" => (value.is_string(), "a string".into()),
                "target" => (
                    value.as_str().is_some_and(|s| matches!(s, "pixels" | "mask" | "filterMask" | "quickMask"))
                        || value.as_object().is_some_and(|o| {
                            o.len() == 1
                                && (o.get("channel").and_then(Value::as_u64).is_some()
                                    || o.get("color").and_then(Value::as_u64).is_some()
                                    || o.get("colors")
                                        .and_then(Value::as_array)
                                        .is_some_and(|a| !a.is_empty() && a.len() <= 64 && a.iter().all(|v| v.as_u64().is_some())))
                        }),
                    "pixels, mask, filterMask, quickMask, {\"channel\":index}, {\"color\":index} or {\"colors\":[indices]}".into(),
                ),
                _ => return Err(bad(format!("unknown parameter `{key}`"))),
            }
        };
        if !valid {
            return Err(bad(format!("`{key}` must be {expected}")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
