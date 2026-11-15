//! Format-neutral adapter from craft-raw metadata into PhotoCraft's develop model.

use crate::RawFormat;
use crate::color::{Calibration, IDENTITY, illuminant_temperature};
use crate::opcodes::{GainMap, MapPolynomial};
use crate::sensor::{BlackLevels, Cfa, Rect, Sensor};

pub(crate) fn format(f: craft_raw::RawFormat) -> RawFormat {
    use craft_raw::RawFormat as F;
    match f {
        F::Dng => RawFormat::Dng,
        F::Cr2 => RawFormat::Cr2,
        F::Cr3 => RawFormat::Cr3,
        F::Nef | F::Nrw => RawFormat::Nef,
        F::Arw => RawFormat::Arw,
        F::Pef => RawFormat::Pef,
        F::Orf => RawFormat::Orf,
        F::Rw2 => RawFormat::Rw2,
        F::Raf => RawFormat::Raf,
        F::CfaTiff => RawFormat::Fff,
        F::Crw | F::Mrw | F::X3f | F::KodakDcs | F::Erf | F::Mef | F::Srw | F::OtherTiff => RawFormat::TiffEp,
    }
}

fn rect(r: craft_raw::Rect) -> Rect {
    Rect::new(r.x, r.y, r.width, r.height)
}

fn gain_maps(image: &craft_raw::RawImage) -> Vec<GainMap> {
    image
        .opcodes
        .list1
        .iter()
        .chain(&image.opcodes.list2)
        .chain(&image.opcodes.list3)
        .filter_map(|op| {
            let craft_raw::Opcode::GainMap { area, points_v, points_h, spacing, origin, map_planes, gains } = op else { return None };
            Some(GainMap {
                top: area.top as usize,
                left: area.left as usize,
                bottom: area.bottom as usize,
                right: area.right as usize,
                plane: area.plane as usize,
                planes: area.planes as usize,
                row_pitch: area.row_pitch as usize,
                col_pitch: area.col_pitch as usize,
                points_v: *points_v as usize,
                points_h: *points_h as usize,
                spacing_v: spacing[0],
                spacing_h: spacing[1],
                origin_v: origin[0],
                origin_h: origin[1],
                map_planes: *map_planes as usize,
                gains: gains.clone(),
            })
        })
        .collect()
}

fn polynomials(image: &craft_raw::RawImage) -> Vec<MapPolynomial> {
    image
        .opcodes
        .list1
        .iter()
        .chain(&image.opcodes.list2)
        .chain(&image.opcodes.list3)
        .filter_map(|op| {
            let craft_raw::Opcode::MapPolynomial { area, coefficients } = op else { return None };
            Some(MapPolynomial {
                top: area.top as usize,
                left: area.left as usize,
                bottom: area.bottom as usize,
                right: area.right as usize,
                plane: area.plane as usize,
                planes: area.planes as usize,
                row_pitch: area.row_pitch as usize,
                col_pitch: area.col_pitch as usize,
                coefficients: coefficients.clone(),
            })
        })
        .collect()
}

fn warnings(image: &craft_raw::RawImage) -> Vec<String> {
    let mut out = Vec::new();
    if image.notices.default_crop_adjusted {
        out.push("raw default crop was invalid, incomplete or out-of-range; adjusted within the active area".into());
    }
    if image.notices.assumed_cfa {
        out.push("raw colour filter phase (Bayer phase) was not recorded or recognized; the decoder's fallback pattern was used".into());
    }
    // Unknown means the decoder has not annotated provenance; it does not mean
    // the file omitted its levels. Report only an explicit fallback decision.
    if image.format != craft_raw::RawFormat::Dng {
        if image.level_sources.black == craft_raw::LevelSource::Fallback {
            out.push("raw black level not recorded in a documented tag; assumed default".into());
        }
        if image.level_sources.white == craft_raw::LevelSource::Fallback {
            out.push("raw white level not recorded in a documented tag; assumed full sample range".into());
        }
    }
    for op in image.opcodes.list1.iter().chain(&image.opcodes.list2).chain(&image.opcodes.list3) {
        if !matches!(op, craft_raw::Opcode::GainMap { .. } | craft_raw::Opcode::MapPolynomial { .. }) {
            out.push("some raw correction opcodes are not applied by PhotoCraft".to_string());
            break;
        }
    }
    if image.notices.gain_table_omitted || image.color.profile.gain_table_map.is_some() {
        out.push("the raw camera profile's ProfileGainTableMap is not applied by PhotoCraft".to_string());
    }
    out
}

/// Convert a validated craft-raw image into the app's neutral processing model.
pub(crate) fn adapt(image: craft_raw::RawImage) -> Sensor {
    let gain_maps = gain_maps(&image);
    let polynomials = polynomials(&image);
    let warnings = warnings(&image);
    let white = [0, 1, 2].map(|i| image.white.get(i).or_else(|| image.white.first()).copied().unwrap_or(65535.0));
    let black = BlackLevels {
        rows: image.black.repeat_rows,
        cols: image.black.repeat_cols,
        values: image.black.values,
        delta_h: image.black.delta_h,
        delta_v: image.black.delta_v,
    };
    let active = rect(image.active_area);
    let crop_rel = image.crop.clipped(active.width, active.height);
    let crop = Rect::new(active.x + crop_rel.x, active.y + crop_rel.y, crop_rel.width, crop_rel.height);
    let cfa = image.cfa.as_ref().map(|c| Cfa { width: c.width, height: c.height, colors: c.pattern.clone(), origin_x: c.origin_x, origin_y: c.origin_y });
    let calibrations = (0..2)
        .filter_map(|i| {
            let color_matrix = image.color.color_matrix[i]?;
            Some(Calibration {
                temperature: illuminant_temperature(u32::from(image.color.illuminant[i])).unwrap_or(5000.0),
                color_matrix,
                forward_matrix: image.color.forward_matrix[i],
                camera_calibration: image.color.camera_calibration[i].unwrap_or(IDENTITY),
            })
        })
        .collect();
    let color = crate::color::ColorInfo {
        calibrations,
        analog_balance: image.color.analog_balance,
        as_shot_neutral: image.color.as_shot_neutral,
        as_shot_white_xy: image.color.as_shot_white_xy,
    };
    let tone_curve = image.color.profile.tone_curve.map_or_else(Vec::new, |c| c.points);
    // Non-DNG TIFF/EP bodies can state their balance through AsShotNeutral.
    // Keep the public camera-WB field populated without changing DNG color policy.
    let camera_wb = image.wb_multipliers.map(|v| v.map(f64::from)).or_else(|| {
        (image.format != craft_raw::RawFormat::Dng)
            .then_some(image.color.as_shot_neutral)
            .flatten()
            .filter(|values| values.iter().all(|value| value.is_finite() && *value > 0.0 && (1.0 / value).is_finite()))
            .map(|values| values.map(|value| 1.0 / value))
    });
    let (data, float_data) = match image.data {
        craft_raw::RawData::U16(v) => (v, None),
        craft_raw::RawData::F32(v) => (Vec::new(), Some(v)),
    };
    let mut sensor = Sensor {
        format: format(image.format),
        make: image.camera.make,
        model: image.camera.model.or(image.camera.unique_model),
        width: image.width,
        height: image.height,
        samples: image.cpp,
        data,
        float_data,
        cfa,
        linearization: None,
        black,
        white,
        active,
        crop,
        color,
        camera_wb,
        orientation: image.orientation.to_exif(),
        baseline_exposure: image.color.baseline_exposure,
        gain_maps,
        polynomials,
        tone_curve,
        black_point: 0.0,
        warnings,
    };
    crate::cameras::apply(&mut sensor, image.bits);
    // DNG's declared white can exceed the stored clipping plateau. Other raw
    // decoders have already chosen their camera-specific or measured levels;
    // their decoded samples must not be silently rescaled a second time.
    if sensor.float_data.is_none() && sensor.format == RawFormat::Dng {
        sensor.lift_clipped_samples();
    }
    sensor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_image() -> craft_raw::RawImage {
        let mut spec = crate::testgen::DngSpec::cfa(64, 64, (0..64 * 64).map(|i| (528 + ((i % 64) * 60 + (i / 64) * 7) as u16 % 4000).min(3567)).collect());
        spec.black = vec![528];
        spec.white = 4095;
        let mut image = craft_raw::decode_with(&spec.build(), &Default::default()).unwrap();
        image.camera.make = None;
        image.camera.model = None;
        image.camera.unique_model = None;
        image
    }

    #[test]
    fn only_declared_fallback_level_provenance_produces_missing_level_notices() {
        let mut image = raw_image();
        image.format = craft_raw::RawFormat::Nef;
        for source in
            [craft_raw::LevelSource::Unknown, craft_raw::LevelSource::Metadata, craft_raw::LevelSource::MaskedPixels, craft_raw::LevelSource::Inferred]
        {
            image.level_sources.black = source;
            image.level_sources.white = source;
            assert!(warnings(&image).iter().all(|warning| !warning.contains("level")), "{source:?}");
        }
        image.level_sources.black = craft_raw::LevelSource::Fallback;
        image.level_sources.white = craft_raw::LevelSource::Fallback;
        let notices = warnings(&image);
        assert!(notices.iter().any(|warning| warning.contains("black level")));
        assert!(notices.iter().any(|warning| warning.contains("white level")));
        image.format = craft_raw::RawFormat::Dng;
        assert!(warnings(&image).iter().all(|warning| !warning.contains("level")), "DNG's documented defaults are not missing camera metadata");
    }

    #[test]
    fn non_dng_decode_preserves_samples_while_dng_clipping_policy_still_applies() {
        let image = raw_image();
        let craft_raw::RawData::U16(stored) = &image.data else { panic!("integer fixture") };
        let original = stored.clone();
        let mut other = image.clone();
        other.format = craft_raw::RawFormat::Nef;
        assert_eq!(adapt(other).data, original, "camera-specific levels must not trigger a second sample rewrite");
        let dng = adapt(image);
        assert_ne!(dng.data, original, "independent early saturation fixture exercises DNG lifting");
        assert!(dng.data.iter().zip(original).all(|(actual, source)| *actual == source || *actual == 4095));
    }

    #[test]
    fn non_dng_as_shot_neutral_populates_camera_balance_and_model_keeps_its_own_identity() {
        let mut image = raw_image();
        image.format = craft_raw::RawFormat::CfaTiff;
        image.camera.model = Some("X2D II 100C".into());
        image.camera.unique_model = Some("Hasselblad X2D II 100C".into());
        image.wb_multipliers = None;
        image.color.as_shot_neutral = Some([0.375, 1., 0.617]);
        let sensor = adapt(image.clone());
        assert_eq!(sensor.model.as_deref(), Some("X2D II 100C"));
        assert_eq!(sensor.camera_wb, Some([1. / 0.375, 1., 1. / 0.617]));
        image.color.as_shot_neutral = Some([0., 1., f64::NAN]);
        assert_eq!(adapt(image).camera_wb, None, "invalid neutral must not create nonfinite balance");
    }

    #[test]
    fn decoder_notices_remain_visible_without_inventing_loss_for_absent_metadata() {
        let mut image = raw_image();
        image.notices = Default::default();
        assert!(warnings(&image).is_empty());
        image.notices.default_crop_adjusted = true;
        assert!(warnings(&image).iter().any(|warning| warning.contains("default crop") && warning.contains("out-of-range")));
        image.notices.default_crop_adjusted = false;
        image.notices.assumed_cfa = true;
        assert!(warnings(&image).iter().any(|warning| warning.contains("colour filter phase") && warning.contains("Bayer phase")));
        image.notices.assumed_cfa = false;
        image.notices.gain_table_omitted = true;
        assert!(warnings(&image).iter().any(|warning| warning.contains("ProfileGainTableMap")));
    }
}
