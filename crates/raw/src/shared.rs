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
    for op in image.opcodes.list1.iter().chain(&image.opcodes.list2).chain(&image.opcodes.list3) {
        if !matches!(op, craft_raw::Opcode::GainMap { .. } | craft_raw::Opcode::MapPolynomial { .. }) {
            out.push("some raw correction opcodes are not applied by PhotoCraft".to_string());
            break;
        }
    }
    if image.color.profile.gain_table_map.is_some() {
        out.push("the raw camera profile's gain table map is not applied by PhotoCraft".to_string());
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
    let (data, float_data) = match image.data {
        craft_raw::RawData::U16(v) => (v, None),
        craft_raw::RawData::F32(v) => (Vec::new(), Some(v)),
    };
    let mut sensor = Sensor {
        format: format(image.format),
        make: image.camera.make,
        model: image.camera.unique_model.or(image.camera.model),
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
        camera_wb: image.wb_multipliers.map(|v| v.map(f64::from)),
        orientation: image.orientation.to_exif(),
        baseline_exposure: image.color.baseline_exposure,
        gain_maps,
        polynomials,
        tone_curve,
        black_point: 0.0,
        warnings,
    };
    crate::cameras::apply(&mut sensor, image.bits);
    if sensor.float_data.is_none() {
        sensor.lift_clipped_samples();
    }
    sensor
}
