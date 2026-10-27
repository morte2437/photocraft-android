use super::controls::remember_edit;
use super::open::RawOpen;
use super::*;

/// The sidecar beside a raw file: `IMG_0001.xmp` for `IMG_0001.CR2`, as Camera Raw names it.
pub fn sidecar_path(path: &str) -> String {
    match path.rsplit_once('.') {
        Some((stem, ext)) if !ext.contains(['/', '\\']) => format!("{stem}.xmp"),
        _ => format!("{path}.xmp"),
    }
}

/// A DNG carries its settings inside the file in Camera Raw; a sidecar is read for one too,
/// but never written beside it.
fn is_dng(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("dng"))
}

/// The settings of the sidecar beside `path`, when there is one that parses: the settings, what
/// the loader skipped, and the sidecar's file name. A sidecar that doesn't parse is ignored (the
/// raw still opens) and reported in the log.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_sidecar(path: &str) -> Option<(CameraRaw, photocraft_io::camera_raw_xmp::LoadReport, String)> {
    let sidecar = sidecar_path(path);
    let Some(bytes) = read_bounded(&sidecar) else {
        log::warn!("{sidecar}: not a readable sidecar of at most {} bytes, ignored", photocraft_io::camera_raw_xmp::MAX_XMP_BYTES);
        return None;
    };
    let mut params = CameraRaw::default();
    match photocraft_io::camera_raw_xmp::load(&mut params, &bytes) {
        Ok(report) => Some((params, report, crate::file_open::display_name(&sidecar))),
        Err(e) => {
            log::warn!("{sidecar}: sidecar ignored: {e}");
            None
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub(super) fn read_sidecar(_: &str) -> Option<(CameraRaw, photocraft_io::camera_raw_xmp::LoadReport, String)> {
    None
}

/// Keep the develop settings beside the raw file (Camera Raw Defaults › Write xmp sidecars):
/// written when the settings aren't the defaults or PhotoCraft's own sidecar is there, so a
/// file reopens as it was developed. `loaded` are the settings the open started from: nothing
/// is written when they didn't change. Not for DNGs, never over a sidecar another application
/// wrote (Lightroom's and Bridge's hold crops, masks, ratings and keywords PhotoCraft can't keep),
/// never for a control or MCP request (it has no authority to write beside the user's files), and
/// never fatal: a folder that can't be written to is reported in a notice.
pub(super) fn write_sidecar(app: &mut PhotocraftApp, raw: &RawOpen, params: &CameraRaw, loaded: Option<&CameraRaw>) {
    if !app.session.prefs().raw_defaults.write_xmp_sidecars || loaded == Some(params) {
        return;
    }
    if app.automation_input || app.session.authorize.is_some() {
        log::info!("{}: sidecar not written for an automation request", raw.name);
        return;
    }
    let Some(path) = raw.path.as_deref().filter(|p| !is_dng(p)) else { return };
    let sidecar = sidecar_path(path);
    let name = crate::file_open::display_name(&sidecar);
    match existing_sidecar_is_ours(&sidecar) {
        Some(false) => {
            let kept = crate::i18n::fmt(tl!("Kept {name}: written by another application"), &[("name", &name)]);
            crate::notices::post(app, kept, Vec::new(), false, None);
            return;
        }
        None if params.is_identity() => return,
        _ => {}
    }
    let bytes = photocraft_io::camera_raw_xmp::save(params).into_bytes();
    let Some(write) = app.services.write.as_mut() else { return };
    if let Err(e) = write(&sidecar, &bytes) {
        crate::notices::error(app, crate::i18n::fmt(tl!("Could not write {name}: {error}"), &[("name", &name), ("error", &e)]));
    }
}

/// `None` when there is no sidecar at `sidecar`, else whether PhotoCraft wrote it. One that
/// can't be read or is too large to be ours counts as another application's.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn existing_sidecar_is_ours(sidecar: &str) -> Option<bool> {
    std::fs::symlink_metadata(sidecar).ok()?;
    Some(read_bounded(sidecar).is_some_and(|b| photocraft_io::camera_raw_xmp::is_photocraft_xmp(&b)))
}

/// The bytes of the regular file at `path` when it holds at most
/// [`photocraft_io::camera_raw_xmp::MAX_XMP_BYTES`]; `None` for anything else (missing, a folder,
/// a pipe or device that might never end, larger). Never reads more than the cap plus one byte.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn read_bounded(path: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let cap = photocraft_io::camera_raw_xmp::MAX_XMP_BYTES;
    // Checked before opening: opening a pipe for reading waits for a writer.
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(cap as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= cap).then_some(bytes)
}

#[cfg(target_arch = "wasm32")]
fn existing_sidecar_is_ours(_: &str) -> Option<bool> {
    None
}

/// Save Settings…: the dialog's settings as a Camera Raw `.xmp` settings file.
pub(super) fn save_settings(app: &mut PhotocraftApp) -> Result<Value, String> {
    let d = app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    let bytes = photocraft_io::camera_raw_xmp::save(&d.params).into_bytes();
    app.pick_save("Camera Raw Settings.xmp", move |app, path| {
        let write = app.services.write.as_mut().ok_or("no writer configured")?;
        write(&path, &bytes)?;
        Ok(json!({"path": path}))
    })
}

/// Load Settings…: applies a Camera Raw `.xmp` settings file's supported settings over the
/// dialog's; the status bar names how many were not supported.
pub(super) fn load_settings(app: &mut PhotocraftApp) -> Result<Value, String> {
    app.camera_raw.as_ref().ok_or(tl!("Camera Raw isn't open"))?;
    app.pick_file_bytes_filtered(&["xmp"], |app, name, bytes| {
        let file = crate::file_open::display_name(&name);
        let d = app.camera_raw.as_mut().ok_or(tl!("Camera Raw isn't open"))?;
        let mut next = d.params.clone();
        let report = photocraft_io::camera_raw_xmp::load(&mut next, &bytes).map_err(|e| format!("{file}: {e}"))?;
        next.validate_changes(&d.original).map_err(|e| format!("{file}: {e}"))?;
        if d.params.point_curve != next.point_curve {
            d.curve_state = Default::default();
        }
        if d.params != next {
            remember_edit(d);
            d.dirty = true;
            d.params = next;
        }
        app.ui.status_error = false;
        app.ui.status = if report.ignored.is_empty() {
            crate::i18n::fmt(tl!("Loaded Camera Raw settings from {file}"), &[("file", &file)])
        } else {
            let template = "Loaded Camera Raw settings from {file} ({n} not supported)";
            crate::i18n::fmt(tl!(template), &[("file", &file), ("n", &report.ignored.len().to_string())])
        };
        Ok(json!({"applied": report.applied, "ignored": report.ignored}))
    })
}

pub(super) fn report_file_error(app: &mut PhotocraftApp, r: Result<Value, String>) {
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status_error = true;
        app.ui.status = e;
    }
}
