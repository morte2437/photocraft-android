//! Camera raw files via `photocraft-raw`: developed to a 16-bit RGB document
//! in ProPhoto RGB (the embedded profile is the built-in ProPhoto-compatible
//! profile), or, for raw variants not decoded yet, the camera's embedded
//! JPEG preview with a warning.
//!
//! Files without colour calibration (everything but DNG) get their colour
//! from the camera's own embedded JPEG: `photocraft-raw` cannot decode JPEG,
//! so the preview is decoded here and handed to [`photocraft_raw::fit_look`],
//! which fits a matrix and tone curve and keeps the neutral fallback when the
//! fit is not clearly better.

use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use craft_cms::{Builtin, Intent, Profile, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType};
pub use photocraft_raw::Demosaic;
use photocraft_raw::{CameraLook, DevelopOptions, Limits, PreviewColorSpace, RawError, Reference, Sensor, WhiteBalance};
pub use photocraft_raw::{EXTENSIONS, Tone, retone};

use crate::flat::image_to_document;
use crate::{ImportResult, IoError};

/// `true` if `bytes` are a camera raw file `photocraft-raw` recognises.
pub fn is_raw(bytes: &[u8]) -> bool {
    photocraft_raw::is_raw(bytes)
}

/// The decode limits shared with the flat codecs.
fn limits() -> Limits {
    let l = codecs::Limits::default();
    Limits { max_width: l.max_width, max_height: l.max_height, max_pixels: l.max_pixels, max_alloc: l.max_alloc }
}

/// A raw file's embedded JPEG preview, turned upright. Its own EXIF orientation wins when it
/// has one; otherwise the raw's IFD0 orientation applies (TIFF-based raws record it there, and
/// their previews are usually stored as the sensor reads out). Developed raws are oriented by
/// `photocraft-raw` and their EXIF carries no orientation, so nothing is turned twice.
fn upright_preview(raw: &[u8], jpeg: &[u8], limits: &photocraft_raw::Limits) -> Result<Image, IoError> {
    // Decode the embedded preview under the caller's limits, not the codec defaults: an
    // embedded JPEG can declare far more decoded pixels than the raw development the caller
    // authorized, so the preview fallback must honor the same policy.
    let codec_limits =
        codecs::Limits { max_width: limits.max_width, max_height: limits.max_height, max_pixels: limits.max_pixels, max_alloc: limits.max_alloc };
    let img = codecs::decode_as_with(Format::Jpeg, jpeg, &codecs::DecodeOptions { keep_orientation: true, limits: codec_limits })?;
    let own = img.meta.exif.as_deref().map_or(1, codecs::exif_orientation);
    let o = if own != 1 { own } else { photocraft_raw::orientation(raw).unwrap_or_else(|| codecs::exif_orientation(raw)) };
    Ok(img.oriented(o)?)
}

/// Camera Raw's Temperature, Tint, Exposure, Shadows, Highlights, Clarity, Texture and Dehaze,
/// applied while developing the sensor data (the open-time Camera Raw dialog) instead of to the
/// developed RGB pixels, the demosaic method, and the crop.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RawTuning {
    /// −100…100, as `photocraft_algo::camera_raw::CameraRaw::temperature`.
    pub temperature: f32,
    /// −100…100, as `CameraRaw::tint`.
    pub tint: f32,
    /// EV, as `CameraRaw::exposure`.
    pub exposure: f32,
    pub demosaic: Demosaic,
    /// −100…100, as `CameraRaw::shadows`: in linear light before the tone curve, as Camera Raw
    /// does for raw files ([`photocraft_raw::Tone`]).
    pub shadows: f32,
    /// −100…100, as `CameraRaw::highlights`.
    pub highlights: f32,
    /// −100…100, as `CameraRaw::clarity`.
    pub clarity: f32,
    /// −100…100, as `CameraRaw::texture`.
    pub texture: f32,
    /// −100…100, as `CameraRaw::dehaze`.
    pub dehaze: f32,
    /// The crop of the raw's sidecar ([`crate::camera_raw_xmp::Crop`], in the stored
    /// orientation): the developed frame is cropped to it once it is upright.
    pub crop: Option<crate::camera_raw_xmp::Crop>,
}

/// Develops a raw file with `tuning` applied at the raw stage: Exposure as develop exposure,
/// Shadows, Highlights, Clarity, Texture and Dehaze in linear light before the tone curve, Temperature / Tint as gains on the
/// camera's as-shot white-balance multipliers (the same gains the Camera Raw filter applies to
/// RGB). The second value is `false` when the file has
/// no as-shot white balance: Temperature / Tint were then not applied, and the caller keeps
/// them as RGB adjustments.
pub fn import_raw_tuned(name: &str, bytes: &[u8], tuning: &RawTuning) -> Result<(ImportResult, bool), IoError> {
    import_raw_tuned_inner(name, bytes, tuning, None)
}

/// [`import_raw_tuned`] from what the open already learned about `bytes` ([`take_kept_decode`],
/// [`RawDecode::new`]): the same pixels without decoding the file and fitting its look again.
/// A `decode` of another file is ignored.
pub fn import_raw_tuned_from(name: &str, bytes: &[u8], tuning: &RawTuning, decode: &RawDecode) -> Result<(ImportResult, bool), IoError> {
    import_raw_tuned_inner(name, bytes, tuning, Some(decode))
}

fn import_raw_tuned_inner(name: &str, bytes: &[u8], tuning: &RawTuning, decode: Option<&RawDecode>) -> Result<(ImportResult, bool), IoError> {
    let format = photocraft_raw::identify(bytes).map(|f| f.name()).unwrap_or("camera raw");
    let exposure = f64::from(tuning.exposure);
    if !exposure.is_finite() || !(-10.0..=10.0).contains(&exposure) {
        return Err(IoError::Raw(RawError::Malformed(format!("exposure {} EV is out of range", tuning.exposure))));
    }
    let fresh;
    let decode = match decode.filter(|d| d.is_of(bytes)) {
        Some(d) => d,
        None => {
            fresh = RawDecode::new(bytes)?;
            &fresh
        }
    };
    let sensor = &decode.sensor;
    let slider = |v: f32| if v.is_finite() { v.clamp(-100.0, 100.0) } else { 0.0 };
    let tone = photocraft_raw::Tone {
        shadows: slider(tuning.shadows),
        highlights: slider(tuning.highlights),
        clarity: slider(tuning.clarity),
        texture: slider(tuning.texture),
        dehaze: slider(tuning.dehaze),
    };
    // The colour rendering the default open used (a look fitted to the camera's JPEG for files
    // without colour calibration), so re-developing changes only what the sliders change.
    let camera_look = decode.look.clone();
    let mut opts = DevelopOptions { limits: limits(), exposure, demosaic: tuning.demosaic, tone, camera_look, ..Default::default() };
    let wb_applied = match sensor.camera_wb {
        Some(m) => {
            let g = wb_gains(tuning.temperature, tuning.tint);
            opts.white_balance = WhiteBalance::Multipliers([m[0] * g[0], m[1] * g[1], m[2] * g[2]]);
            true
        }
        None => false,
    };
    let mut dev = photocraft_raw::develop_sensor(sensor, &opts).map_err(IoError::Raw)?;
    let cropped = match tuning.crop {
        Some(crop) => {
            let o = if opts.orient { dev.info.orientation } else { 1 };
            crop_developed(&mut dev, crop.oriented(o))?;
            format!(", cropped to {}×{}", dev.width, dev.height)
        }
        None => String::new(),
    };
    let wb = if wb_applied {
        format!("as-shot white balance, temperature {:+.0}, tint {:+.0}", tuning.temperature, tuning.tint)
    } else {
        "estimated white balance".into()
    };
    let tone = [("shadows", tone.shadows), ("highlights", tone.highlights), ("clarity", tone.clarity), ("texture", tone.texture), ("dehaze", tone.dehaze)]
        .iter()
        .filter(|(_, v)| *v != 0.0)
        .map(|(k, v)| format!(", {k} {v:+.0}"))
        .collect::<String>();
    let summary = format!("developed in RAW Develop (exposure {:+.2} EV{tone}, {wb}, {} demosaic{cropped})", tuning.exposure, tuning.demosaic.id());
    Ok((developed_document(name, bytes, format, dev, &summary)?, wb_applied))
}

/// Crops the upright developed frame to `crop` (fractions of it), as Camera Raw crops a raw.
fn crop_developed(dev: &mut photocraft_raw::Developed, crop: crate::camera_raw_xmp::Crop) -> Result<(), IoError> {
    let (w, h) = (dev.width as usize, dev.height as usize);
    let bad = || IoError::Raw(RawError::Malformed(format!("the developed {w}x{h} frame holds fewer samples than its size")));
    let Some((x0, y0, x1, y1)) = crop.pixels(w, h) else { return Ok(()) };
    let mut rgb = Vec::with_capacity((x1 - x0) * (y1 - y0) * 3);
    for y in y0..y1 {
        let row = y.checked_mul(w).ok_or_else(bad)?;
        rgb.extend_from_slice(dev.rgb.get((row + x0) * 3..(row + x1) * 3).ok_or_else(bad)?);
    }
    dev.rgb = rgb;
    dev.width = u32::try_from(x1 - x0).map_err(|_| bad())?;
    dev.height = u32::try_from(y1 - y0).map_err(|_| bad())?;
    Ok(())
}

/// The tone curve `bytes` develop with by default: the fitted look's for a file without colour
/// calibration (see [`import_raw_with`]), else the camera profile's (empty: none, or not a raw
/// this crate decodes). For previews of the open-time raw-stage sliders
/// ([`photocraft_raw::retone`]), in linear-light knots.
pub fn tone_curve(bytes: &[u8]) -> Vec<[f32; 2]> {
    RawDecode::new(bytes).map(|d| d.tone_curve).unwrap_or_default()
}

/// A raw file decoded once for the open-time Camera Raw dialog: the sensor data, the look fitted
/// to the camera's JPEG (for a file without colour calibration, see [`default_look`]) and the
/// tone curve the default develop uses. The dialog's preview and its Open re-develop
/// ([`import_raw_tuned_from`]) share one instead of decoding and fitting the file again each
/// (the sensor is 2 bytes per pixel: drop it when the dialog closes).
pub struct RawDecode {
    fingerprint: Fingerprint,
    sensor: Sensor,
    look: Option<CameraLook>,
    tone_curve: Vec<[f32; 2]>,
}

impl std::fmt::Debug for RawDecode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Not the sensor data: tens of millions of samples.
        f.debug_struct("RawDecode").field("fingerprint", &self.fingerprint).field("look", &self.look.is_some()).finish_non_exhaustive()
    }
}

impl RawDecode {
    /// Decodes `bytes` and fits its look, as a default open does.
    pub fn new(bytes: &[u8]) -> Result<RawDecode, IoError> {
        let sensor = decode_sensor(bytes, &limits())?;
        let look = default_look(bytes, &sensor);
        Ok(RawDecode::from_parts(bytes, sensor, look))
    }

    fn from_parts(bytes: &[u8], sensor: Sensor, look: Option<CameraLook>) -> RawDecode {
        let tone_curve = match &look {
            Some(look) => look.tone.as_ref().map(look_knots).unwrap_or_default(),
            None => sensor.tone_curve.clone(),
        };
        RawDecode { fingerprint: Fingerprint::of(bytes), sensor, look, tone_curve }
    }

    /// The tone curve a default develop uses, as [`tone_curve`] returns it.
    pub fn tone_curve(&self) -> &[[f32; 2]] {
        &self.tone_curve
    }

    /// `true` if this was decoded from `bytes`.
    fn is_of(&self, bytes: &[u8]) -> bool {
        self.fingerprint == Fingerprint::of(bytes)
    }
}

/// The sensor data of `bytes` under `limits`.
fn decode_sensor(bytes: &[u8], limits: &Limits) -> Result<Sensor, IoError> {
    DECODES.with(|n| n.set(n.get() + 1));
    photocraft_raw::decode(bytes, limits).map_err(IoError::Raw)
}

thread_local! {
    static DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many raw files this thread has decoded so far (for tests that count decodes).
#[doc(hidden)]
pub fn decodes_on_this_thread() -> usize {
    DECODES.with(std::cell::Cell::get)
}

/// A file's length and a 64-bit digest of its bytes: tells the file a [`RawDecode`] was made
/// from from any other (not a cryptographic hash).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Fingerprint(usize, u64);

impl Fingerprint {
    fn of(bytes: &[u8]) -> Fingerprint {
        const K: u64 = 0x9E37_79B9_7F4A_7C15;
        let mix = |lane: u64, word: u64| (lane ^ word).wrapping_mul(K).rotate_left(29);
        let mut lanes = [0x243F_6A88_85A3_08D3u64, 0x1319_8A2E_0370_7344, 0xA409_3822_299F_31D0, 0x082E_FA98_EC4E_6C89];
        let (blocks, rest) = bytes.as_chunks::<32>();
        let mut absorb = |block: &[u8; 32]| {
            for (lane, word) in lanes.iter_mut().zip(block.as_chunks::<8>().0) {
                *lane = mix(*lane, u64::from_le_bytes(*word));
            }
        };
        blocks.iter().for_each(&mut absorb);
        let mut tail = [0u8; 32];
        tail[..rest.len()].copy_from_slice(rest);
        absorb(&tail);
        let digest = lanes.iter().fold(bytes.len() as u64, |acc, &lane| mix(acc, lane));
        Fingerprint(bytes.len(), digest)
    }
}

/// The decodes of the last opens that await their Camera Raw dialog (see
/// [`ImportOptions::keep_raw`](crate::ImportOptions::keep_raw)): at most two, so several raws
/// opened at once each find theirs, and the oldest goes first. The dialog takes its own out.
static KEPT: Mutex<Vec<Arc<RawDecode>>> = Mutex::new(Vec::new());

const KEPT_MAX: usize = 2;

fn kept() -> std::sync::MutexGuard<'static, Vec<Arc<RawDecode>>> {
    KEPT.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Hands over (removes) the decode a default open of exactly `bytes` kept for its dialog, if it
/// is still there. Whoever takes it owns the sensor data: dropping it frees it.
pub fn take_kept_decode(bytes: &[u8]) -> Option<Arc<RawDecode>> {
    let want = Fingerprint::of(bytes);
    let mut kept = kept();
    let at = kept.iter().position(|d| d.fingerprint == want)?;
    Some(kept.remove(at))
}

/// A fitted look's tone curve as knots for [`photocraft_raw::retone`] (which reads them
/// log-linearly): sampled densely enough that the two agree within a fraction of a code value.
fn look_knots(t: &photocraft_raw::ToneCurve) -> Vec<[f32; 2]> {
    const N: usize = 256;
    (1..=N).map(|i| (i as f64 / N as f64).powi(2)).map(|x| [x as f32, t.eval(x) as f32]).collect()
}

/// The look a default open develops `bytes` with: fitted to the embedded JPEG when the file has
/// no colour calibration, is not monochrome and the fit clearly helps (as [`import_raw_with`]
/// does), else `None`. It is fitted at the camera's own white balance and exposure, so a
/// re-develop's sliders are not fitted away.
fn default_look(bytes: &[u8], sensor: &Sensor) -> Option<CameraLook> {
    if !sensor.color.calibrations.is_empty() || sensor.is_monochrome() {
        return None;
    }
    fit_preview_look(bytes, sensor, &DevelopOptions { limits: limits(), ..Default::default() }).ok()
}

/// "Canon" + "Canon EOS 80D" and "NIKON CORPORATION" + "NIKON D3200" read as the model alone:
/// the model already starts with the make's first word.
fn camera_name(make: Option<&str>, model: Option<&str>) -> String {
    match (make, model) {
        (Some(make), Some(model))
            if make.split_whitespace().next().is_some_and(|brand| model.to_ascii_lowercase().starts_with(&brand.to_ascii_lowercase())) =>
        {
            model.to_string()
        }
        (make, model) => [make, model].into_iter().flatten().collect::<Vec<_>>().join(" "),
    }
}

/// Camera Raw's white-balance gains for Temperature / Tint (−100…100), the same the Camera
/// Raw filter applies ([`photocraft_algo::camera_raw::white_balance_gains`]).
fn wb_gains(temperature: f32, tint: f32) -> [f64; 3] {
    photocraft_algo::camera_raw::white_balance_gains(temperature.clamp(-100.0, 100.0), tint.clamp(-100.0, 100.0)).map(f64::from)
}

/// A developed raw as a 16-bit ProPhoto document, with a note saying how it was developed.
fn developed_document(name: &str, bytes: &[u8], format: &str, dev: photocraft_raw::Developed, summary: &str) -> Result<ImportResult, IoError> {
    let mut img = Image::from_u16(dev.width, dev.height, ChannelLayout::Rgb, &dev.rgb)?;
    // The raw's own EXIF (camera, lens, exposure, date), for File Info's Camera Data. It carries
    // no orientation: the developed image is already upright.
    img.meta.exif = codecs::raw_exif(bytes);
    let mut r = image_to_document(name, &img)?;
    r.document.icc_profile = Some(Arc::new(craft_cms::Builtin::ProPhotoCompat.profile().to_bytes().to_vec()));
    let camera = camera_name(dev.info.make.as_deref(), dev.info.model.as_deref());
    r.warnings.push(format!(
        "{format}{} {summary} into 16-bit {}",
        if camera.is_empty() { String::new() } else { format!(" from {camera}") },
        photocraft_raw::OUTPUT_SPACE
    ));
    r.warnings.extend(dev.warnings);
    Ok(r)
}

/// Develops a raw file with the default settings.
pub fn import_raw(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    import_raw_demosaic(name, bytes, Demosaic::default())
}

/// Develops a raw file with the default settings and the `demosaic` method.
pub fn import_raw_demosaic(name: &str, bytes: &[u8], demosaic: Demosaic) -> Result<ImportResult, IoError> {
    import_raw_with(name, bytes, &DevelopOptions { limits: limits(), demosaic, ..Default::default() })
}

/// [`import_raw_demosaic`] that also keeps the sensor data and the fitted look for the file's
/// Camera Raw dialog ([`take_kept_decode`]).
pub fn import_raw_demosaic_keeping(name: &str, bytes: &[u8], demosaic: Demosaic) -> Result<ImportResult, IoError> {
    import_raw_with_inner(name, bytes, &DevelopOptions { limits: limits(), demosaic, ..Default::default() }, true)
}

/// Smallest preview long edge preferred for the colour fit (a smaller preview decodes faster;
/// the fit grid is at most `PROXY_EDGE` cells and wants about two pixels per cell).
const FIT_PREVIEW_EDGE: u32 = 2 * photocraft_raw::PROXY_EDGE as u32;

/// The colour space of a raw's decoded preview when it is not sRGB: its own ICC profile, else the
/// one the raw's EXIF states (a Sony body set to Adobe RGB writes no profile into the JPEG).
/// `None` means sRGB.
fn preview_profile(raw: &[u8], icc: Option<&[u8]>) -> Result<Option<Cow<'static, Profile>>, String> {
    let src = match icc {
        Some(icc) => Cow::Owned(Profile::parse(icc).map_err(|e| format!("the preview's ICC profile does not parse: {e}"))?),
        None if photocraft_raw::embedded_preview_color_space(raw) == Some(PreviewColorSpace::AdobeRgb) => Cow::Borrowed(Builtin::AdobeRgbCompat.profile()),
        None => return Ok(None),
    };
    Ok((!src.same_colors(Builtin::Srgb.profile())).then_some(src))
}

/// Fits the colour of a decoded raw to the camera's embedded JPEG (see
/// [`photocraft_raw::fit_look`]): picks the smallest preview at least
/// `FIT_PREVIEW_EDGE` pixels long (else the largest), decodes it, turns it upright and
/// converts it to sRGB when it is in another colour space ([`preview_profile`]). `Err` says
/// why there is no fit.
pub fn fit_preview_look(raw: &[u8], sensor: &Sensor, opts: &DevelopOptions) -> Result<CameraLook, String> {
    let jpeg = photocraft_raw::embedded_jpeg_preview_covering(raw, FIT_PREVIEW_EDGE)
        .or_else(|| photocraft_raw::embedded_jpeg_preview(raw))
        .ok_or("no embedded JPEG preview")?;
    let img = upright_preview(raw, &jpeg, &opts.limits).map_err(|e| format!("the embedded preview does not decode: {e}"))?;
    if img.layout() != ChannelLayout::Rgb || img.sample_type() != SampleType::U8 {
        return Err("the embedded preview is not 8-bit RGB".into());
    }
    let (w, h) = img.dimensions();
    let mut rgb: Cow<[u8]> = Cow::Borrowed(img.data());
    if let Some(src) = preview_profile(raw, img.icc.as_deref())? {
        let t = Transform::new(&src, Builtin::Srgb.profile(), Intent::RelativeColorimetric, false).map_err(|e| e.to_string())?;
        let mut out = vec![0u8; rgb.len()];
        t.convert_u8(&rgb, 3, &mut out, 3, false).map_err(|e| e.to_string())?;
        rgb = Cow::Owned(out);
    }
    photocraft_raw::fit_look(sensor, opts, &Reference { width: w, height: h, rgb: &rgb }).map_err(|e| e.to_string())
}

/// A sensor and the look fitted to it, as [`develop_fitted`] hands them back.
type Kept = (Sensor, Option<CameraLook>);

/// Decodes and develops; a file without colour calibration gets a look fitted to its preview.
/// With `keep`, also hands back the sensor and the look it fitted (the default look, when
/// `opts` named none) instead of freeing them.
fn develop_fitted(bytes: &[u8], opts: &DevelopOptions, keep: bool) -> Result<(photocraft_raw::Developed, Option<Kept>), RawError> {
    let sensor = decode_sensor(bytes, &opts.limits).map_err(|e| match e {
        IoError::Raw(e) => e,
        e => RawError::Malformed(e.to_string()),
    })?;
    let mut opts = Cow::Borrowed(opts);
    let named_look = opts.camera_look.is_some();
    if opts.camera_look.is_none()
        && sensor.color.calibrations.is_empty()
        && !sensor.is_monochrome()
        && let Ok(look) = fit_preview_look(bytes, &sensor, &opts)
    {
        opts.to_mut().camera_look = Some(look);
    }
    let dev = photocraft_raw::develop_sensor(&sensor, &opts)?;
    let kept = (keep && !named_look).then(|| (sensor, opts.into_owned().camera_look));
    Ok((dev, kept))
}

/// The camera's embedded JPEG, upright, for opening in place of sensor data that can't be
/// developed. A preview without an ICC profile of its own is tagged with the colour space the
/// raw's EXIF states (a Sony body set to Adobe RGB writes no profile into the JPEG), as the colour
/// fit reads it ([`preview_profile`]); otherwise it opens as untagged sRGB.
fn preview_image(raw: &[u8], jpeg: &[u8], limits: &photocraft_raw::Limits) -> Result<Image, IoError> {
    let mut img = upright_preview(raw, jpeg, limits)?;
    if img.icc.is_none() && photocraft_raw::embedded_preview_color_space(raw) == Some(PreviewColorSpace::AdobeRgb) {
        img.icc = Some(Builtin::AdobeRgbCompat.profile().to_bytes().to_vec());
    }
    Ok(img)
}

/// A preview opened in place of the raw, with `note` saying why first among the warnings.
fn preview_document(name: &str, img: &Image, note: String) -> Result<ImportResult, IoError> {
    let mut r = image_to_document(name, img)?;
    r.warnings.insert(0, note);
    Ok(r)
}

/// In the import note of a raw developed from its sensor data with the default settings (not of
/// a preview fallback): what the open-time Camera Raw dialog looks for.
pub const DEVELOPED_NOTE: &str = "developed with default settings";

/// The demosaic method a default open developed with, read back from its [`DEVELOPED_NOTE`]
/// warning: the open-time dialog starts from it, whatever the preference says by the time the
/// (background) open lands.
pub fn developed_demosaic(warnings: &[String]) -> Option<Demosaic> {
    let note = warnings.iter().find(|w| w.contains(DEVELOPED_NOTE))?;
    let after = &note[note.find(DEVELOPED_NOTE)? + DEVELOPED_NOTE.len()..];
    let id = after.trim_start().strip_prefix('(')?.split(" demosaic").next()?;
    Demosaic::from_id(id)
}

/// Develops a raw file with explicit settings. Without `opts.camera_look`, a file without
/// colour calibration gets one fitted to its embedded JPEG when that clearly helps.
pub fn import_raw_with(name: &str, bytes: &[u8], opts: &DevelopOptions) -> Result<ImportResult, IoError> {
    import_raw_with_inner(name, bytes, opts, false)
}

fn import_raw_with_inner(name: &str, bytes: &[u8], opts: &DevelopOptions, keep: bool) -> Result<ImportResult, IoError> {
    let format = photocraft_raw::identify(bytes).map(|f| f.name()).unwrap_or("camera raw");
    match develop_fitted(bytes, opts, keep) {
        Ok((dev, kept_parts)) => {
            if let Some((sensor, look)) = kept_parts {
                let mut kept = kept();
                if kept.len() == KEPT_MAX {
                    kept.remove(0);
                }
                kept.push(Arc::new(RawDecode::from_parts(bytes, sensor, look)));
            }
            let summary = format!("{DEVELOPED_NOTE} ({} demosaic, as-shot white balance)", opts.demosaic.id());
            developed_document(name, bytes, format, dev, &summary)
        }
        Err(RawError::Unsupported(reason)) => {
            let jpeg = photocraft_raw::embedded_jpeg_preview(bytes).map(|jpeg| (jpeg,));
            // A DNG 1.7 converter keeps only its small thumbnail as JPEG and writes the larger
            // previews as JPEG XL, like the raw data (#8125): open the largest of either kind.
            let jpeg_img = jpeg.map(|(jpeg,)| preview_image(bytes, &jpeg, &opts.limits).map(|img| (jpeg, img)));
            let jpeg_area = jpeg_img.as_ref().and_then(|r| r.as_ref().ok()).map_or(0, |(_, img)| u64::from(img.width()) * u64::from(img.height()));
            let jxl = photocraft_raw::embedded_jxl_preview(bytes, &opts.limits).filter(|p| u64::from(p.width) * u64::from(p.height) > jpeg_area);
            let (img, kind, (w, h)) = match (jxl, jpeg_img) {
                (Some(p), _) => {
                    let size = (p.width, p.height);
                    (jxl_preview_image(bytes, p)?, "JPEG XL", size)
                }
                (None, Some(Ok((_, img)))) => {
                    let size = img.dimensions();
                    (img, "JPEG", size)
                }
                (None, Some(Err(error))) => return Err(error),
                (None, None) => return Err(IoError::Raw(RawError::Unsupported(reason))),
            };
            let depth = if img.sample_type() == SampleType::U8 { "8-bit" } else { "16-bit" };
            let note = format!(
                "{format}: {reason} is not supported yet; opened the camera's embedded {w}x{h} {kind} preview instead ({depth}, not the raw sensor data)"
            );
            preview_document(name, &img, note)
        }
        // A damaged DNG can still carry a valid JPEG, but opening that thumbnail as if it were
        // the photograph hides a corrupt raw source. Keep preview fallback for other camera
        // formats whose independent preview is the established recovery path.
        Err(RawError::Malformed(reason)) if format == "DNG" => Err(IoError::Raw(RawError::Malformed(reason))),
        Err(RawError::Malformed(reason)) => match photocraft_raw::embedded_jpeg_preview(bytes).map(|jpeg| preview_image(bytes, &jpeg, &opts.limits)) {
            Some(Ok(img)) => {
                let (w, h) = img.dimensions();
                let note = format!(
                    "{format}: the raw sensor data is damaged ({reason}); opened the camera's embedded {w}x{h} JPEG preview instead (8-bit, not the raw sensor data)"
                );
                preview_document(name, &img, note)
            }
            _ => Err(IoError::Raw(RawError::Malformed(reason))),
        },
        Err(e) => Err(IoError::Raw(e)),
    }
}

/// A decoded JPEG XL preview as an image: in its stated colour space (sRGB when it states
/// none), turned upright by the raw's orientation (DNG previews are stored as the raw is).
fn jxl_preview_image(raw: &[u8], p: photocraft_raw::DecodedPreview) -> Result<Image, IoError> {
    let img = if p.bits == 8 {
        Image::from_u8(p.width, p.height, ChannelLayout::Rgb, p.rgb.iter().map(|&v| v.min(255) as u8).collect())?
    } else {
        let max = f64::from((1u32 << p.bits.clamp(1, 16)) - 1);
        let rgb: Vec<u16> = p.rgb.iter().map(|&v| (f64::from(v).min(max) * 65535.0 / max).round() as u16).collect();
        Image::from_u16(p.width, p.height, ChannelLayout::Rgb, &rgb)?
    };
    let icc = match p.color_space {
        Some(PreviewColorSpace::AdobeRgb) => Some(Builtin::AdobeRgbCompat.profile().to_bytes().to_vec()),
        Some(PreviewColorSpace::ProPhoto) => Some(Builtin::ProPhotoCompat.profile().to_bytes().to_vec()),
        Some(PreviewColorSpace::Srgb) | None => None,
    };
    Ok(img.with_icc(icc).oriented(photocraft_raw::orientation(raw).unwrap_or(1))?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A caller's tighter limits hold for the sensor decode, not only for the develop after it (#9462).
    #[test]
    fn import_raw_with_keeps_the_callers_decode_limits() {
        use photocraft_raw::testgen::{DngSpec, mosaic, scene};
        let (w, h) = (40, 24);
        let bytes = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build();
        assert!(import_raw_with("a.dng", &bytes, &DevelopOptions::default()).is_ok());
        let tight = DevelopOptions { limits: Limits { max_width: 8, max_height: 8, max_pixels: 64, max_alloc: 1 << 20 }, ..DevelopOptions::default() };
        assert!(matches!(import_raw_with("a.dng", &bytes, &tight), Err(IoError::Raw(RawError::LimitExceeded(_)))));
    }

    #[test]
    fn the_developed_note_says_which_demosaic_method_was_used() {
        let note = format!("NEF: {DEVELOPED_NOTE} (mhc demosaic, as-shot white balance) into 16-bit ProPhoto RGB");
        assert_eq!(developed_demosaic(&["unrelated".to_string(), note]), Some(Demosaic::Mhc));
        assert_eq!(developed_demosaic(&[format!("{DEVELOPED_NOTE} (amaze demosaic)")]), None);
        assert_eq!(developed_demosaic(&["opened the embedded preview".to_string()]), None);
    }

    #[test]
    fn camera_names_drop_a_repeated_make() {
        assert_eq!(camera_name(Some("Canon"), Some("Canon EOS 80D")), "Canon EOS 80D");
        assert_eq!(camera_name(Some("NIKON CORPORATION"), Some("NIKON D3200")), "NIKON D3200");
        assert_eq!(camera_name(Some("SONY"), Some("ILCE-7M3")), "SONY ILCE-7M3");
        assert_eq!(camera_name(None, Some("X")), "X");
        assert_eq!(camera_name(Some("Panasonic"), Some("DC-G9")), "Panasonic DC-G9");
    }

    /// A Sony body set to Adobe RGB says so only in the raw's EXIF (ColorSpace Uncalibrated, InteropIndex "R03"):
    /// its preview, which has no ICC profile, is read as Adobe RGB for the colour fit, not as sRGB.
    #[test]
    fn a_preview_without_a_profile_takes_the_raws_exif_colour_space() {
        use photocraft_raw::testgen::{TiffBuilder, Val};
        let raw = |color_space: u16, index: &str| {
            let mut t = TiffBuilder::default();
            let interop = t.ifd(vec![(0x0001, Val::Ascii(index.into()))]);
            let exif = t.ifd(vec![(0xA001, Val::Short(vec![color_space])), (0xA005, Val::Ifds(vec![interop]))]);
            t.chain = vec![t.ifd(vec![(271, Val::Ascii("SONY".into())), (34665, Val::Ifds(vec![exif]))])];
            t.build()
        };
        let adobe = raw(0xFFFF, "R03");
        let srgb = raw(1, "R98");
        assert!(preview_profile(&adobe, None).unwrap().unwrap().same_colors(Builtin::AdobeRgbCompat.profile()));
        assert!(preview_profile(&srgb, None).unwrap().is_none());
        assert!(preview_profile(b"not a raw", None).unwrap().is_none(), "unstated: sRGB");
        // the preview's own profile wins over the raw's EXIF
        let srgb_icc = Builtin::Srgb.profile().to_bytes().to_vec();
        assert!(preview_profile(&adobe, Some(srgb_icc.as_slice())).unwrap().is_none());
        let p3_icc = Builtin::DisplayP3.profile().to_bytes().to_vec();
        assert!(preview_profile(&srgb, Some(p3_icc.as_slice())).unwrap().unwrap().same_colors(Builtin::DisplayP3.profile()));
        assert!(preview_profile(&adobe, Some(b"not an ICC profile")).is_err());
    }
}
