//! Computational-photography automation: File › Automate › Photomerge, Merge to HDR Pro and
//! Crop and Straighten Photos. The registration and blending code
//! ([`photocraft_algo::panorama`]) is shared with Edit › Auto-Align / Auto-Blend Layers
//! (`align_cmds`).
//!
//! Inputs are files (`"paths"`: an array or a folder) or, with `"useOpenDocuments": true`, the
//! open documents (Photoshop's *Add Open Files*). Each command creates new documents; the
//! sources are untouched.

use photocraft_algo::exif;
use photocraft_algo::hdr::{self, MergeOptions, ToneMethod};
use photocraft_algo::panorama::{self, AlignOptions, Alignment, Layout, Placement, RoiImage};
use photocraft_algo::scancrop;
use photocraft_algo::tone::HdrToning;
use photocraft_algo::transform::{Homography, Interp, warp_surface};
use photocraft_algo::warp::warp_mesh_surface;
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Layer, LayerContent, LayerMask, Size};
use photocraft_geom::{Rect, TILE_SIZE};
use photocraft_raster::{Surface, to_rgba};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{file_name, flattened, flattened_checked, import, import_with_options, list_images, read_file, read_file_limited};
use crate::{EngineError, Result, Session};

/// Wall-clock timer for the `ms` fields of results (always 0 on the web, where
/// `std::time::Instant` is unavailable).
pub(crate) struct Stopwatch(#[cfg(not(target_arch = "wasm32"))] std::time::Instant);

impl Stopwatch {
    pub(crate) fn start() -> Self {
        Stopwatch(
            #[cfg(not(target_arch = "wasm32"))]
            std::time::Instant::now(),
        )
    }
    pub(crate) fn ms(&self) -> f64 {
        #[cfg(not(target_arch = "wasm32"))]
        return self.0.elapsed().as_secs_f64() * 1000.0;
        #[cfg(target_arch = "wasm32")]
        0.0
    }
}

/// Longest side used for feature registration.
pub(crate) const REGISTER_SIDE: usize = 1200;

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

// ---------- sources ----------

/// One input image, flattened.
pub(crate) struct Source {
    pub name: String,
    pub surf: Surface,
    pub w: usize,
    pub h: usize,
    pub exif: exif::CameraInfo,
}

/// Loaded sources, the common pixel format and the first source's ICC profile.
type Loaded = (Vec<Source>, PixelFormat, Option<std::sync::Arc<Vec<u8>>>);

/// Pixel bytes every surface `doc` holds, counting a tile shared between surfaces once.
fn document_pixels(doc: &Document) -> usize {
    let mut seen = std::collections::HashSet::new();
    let mut total = 0usize;
    doc.for_each_surface(|surface| {
        for (_, tile) in surface.tiles() {
            if seen.insert(std::sync::Arc::as_ptr(tile) as usize) {
                total = total.saturating_add(tile.len());
            }
        }
    });
    total
}

/// Bytes a `w` × `h` surface allocates in `bpp`: whole 256 px tiles, as [`Surface`] stores them.
fn surface_bytes(w: usize, h: usize, bpp: usize) -> usize {
    if w == 0 || h == 0 {
        return 0;
    }
    let side = TILE_SIZE as usize;
    w.div_ceil(side).saturating_mul(h.div_ceil(side)).saturating_mul(side.saturating_mul(side)).saturating_mul(bpp)
}

/// `bytes` for people ("12 MiB", "900 KiB", "17 B").
fn bytes_text(bytes: usize) -> String {
    if bytes >= 1 << 20 {
        format!("{} MiB", bytes >> 20)
    } else if bytes >= 1 << 10 {
        format!("{} KiB", bytes >> 10)
    } else {
        format!("{bytes} B")
    }
}

/// Keeps a Photomerge's sources inside the command's memory budget while they are loaded.
///
/// `None` (Merge to HDR Pro) has no budget and loads as before. With a budget, a file is read
/// only up to what is left, its import is lowered to that remainder through the importer's own
/// preflight, and the decoded document plus the surfaces flattening allocates are charged
/// before the source is flattened.
struct SourceLoad {
    budget: Option<usize>,
    /// Pixels of the flattened sources kept so far.
    retained: usize,
    /// Sources admitted so far (for the error message).
    count: usize,
}

impl SourceLoad {
    fn new(budget: Option<usize>) -> Self {
        Self { budget, retained: 0, count: 0 }
    }

    /// What one more source needs while it is flattened: the document's own pixels, the source
    /// surface, the flatten's working and output surfaces, and a file's encoded bytes.
    fn needed(&self, doc: &Document, bpp: usize, encoded: usize) -> usize {
        let b = doc.bounds();
        let output = surface_bytes(b.width() as usize, b.height() as usize, bpp);
        self.retained.saturating_add(encoded).saturating_add(document_pixels(doc)).saturating_add(output.saturating_mul(2))
    }

    fn exceeds(&self, cmd: &str, doc: &Document, bpp: usize, encoded: usize, budget: usize) -> EngineError {
        let b = doc.bounds();
        crate::allocation::error(
            cmd,
            &format!(
                "loading {} source image(s) plus another {}×{} one needs about {}, which exceeds the {} budget; use fewer or smaller images, or raise \"memoryBudget\"",
                self.count,
                b.width(),
                b.height(),
                bytes_text(self.needed(doc, bpp, encoded)),
                bytes_text(budget)
            ),
        )
    }

    /// Admits and charges one `doc` flattened in `bpp`, or fails before the flatten allocates.
    fn admit(&mut self, cmd: &str, doc: &Document, bpp: usize, encoded: usize) -> Result<()> {
        let Some(budget) = self.budget else {
            self.count += 1;
            return Ok(());
        };
        if self.needed(doc, bpp, encoded) > budget {
            return Err(self.exceeds(cmd, doc, bpp, encoded, budget));
        }
        let b = doc.bounds();
        self.retained = self.retained.saturating_add(surface_bytes(b.width() as usize, b.height() as usize, bpp));
        self.count += 1;
        Ok(())
    }

    /// What is left of the budget for the next source.
    fn remaining(&self, cmd: &str) -> Result<usize> {
        let Some(budget) = self.budget else { return Ok(usize::MAX) };
        budget
            .checked_sub(self.retained)
            .filter(|left| *left > 0)
            .ok_or_else(|| crate::allocation::error(cmd, &format!("the sources loaded so far use the whole {} budget", bytes_text(budget))))
    }

    /// Reads and imports one file, bounding the encoded read and the decode by what is left.
    /// `None` (Merge to HDR Pro) reads and imports as before.
    fn load_file(&mut self, s: &Session, path: &str, cmd: &str) -> Result<(String, Document, usize)> {
        let name = file_name(path);
        if self.budget.is_none() {
            let bytes = read_file(path)?;
            return Ok((name.clone(), import(&name, &bytes)?, bytes.len()));
        }
        let left = self.remaining(cmd)?;
        let bytes = read_file_limited(path, u64::try_from(left).unwrap_or(u64::MAX)).map_err(|e| match e.kind() {
            // Only a file over what is left, or one memory cannot hold, is a memory error; a
            // missing or unreadable file is a file error, as without a budget.
            std::io::ErrorKind::FileTooLarge | std::io::ErrorKind::OutOfMemory => {
                crate::allocation::error(cmd, &format!("{e} (only {} is left of the budget for a source file's bytes)", bytes_text(left)))
            }
            _ => EngineError::Other(format!("{path}: {e}")),
        })?;
        let available = left.saturating_sub(bytes.len());
        let opts = self.import_options(s, &name, &bytes, available, cmd)?;
        let doc = import_with_options(&name, &bytes, &opts).map_err(|e| match e {
            // The caller limit is this command's remainder, not the machine's hard limit.
            EngineError::PsdHardBudget { required, limit, .. } if limit == u64::try_from(available).unwrap_or(u64::MAX) => crate::allocation::error(
                cmd,
                &format!(
                    "decoding {name} needs about {}, which exceeds the {} left; use fewer or smaller images, or raise \"memoryBudget\"",
                    bytes_text(usize::try_from(required).unwrap_or(usize::MAX)),
                    bytes_text(available)
                ),
            ),
            e => e,
        })?;
        Ok((name, doc, bytes.len()))
    }

    /// The importer options that lower this import to what is left after the retained sources
    /// and the encoded buffer: the importer's own PSD/TIFF preflight, or a flat decode limit.
    /// The decoded document and the flatten are still charged by [`Self::admit`].
    fn import_options(&self, s: &Session, name: &str, bytes: &[u8], available: usize, cmd: &str) -> Result<photocraft_io::ImportOptions> {
        use photocraft_io::ImportPreflight;
        let mut opts = crate::jobs::import_options(s.prefs());
        if available == 0 {
            return Err(crate::allocation::error(
                cmd,
                &format!("the sources already loaded and {name}'s {} encoded bytes use the whole budget", bytes_text(bytes.len())),
            ));
        }
        let limit = u64::try_from(available).unwrap_or(u64::MAX);
        match photocraft_io::import_preflight(name, bytes) {
            ImportPreflight::Psd => opts.psd_max_pixel_bytes = limit,
            ImportPreflight::Tiff => {
                self.admit_tiff_header(name, bytes, available, cmd)?;
                opts.psd_max_pixel_bytes = limit;
            }
            ImportPreflight::Flat => {
                // The decoded buffer, the document built from it and the flatten's output are
                // alive together, so keep a third of the remainder for each.
                let decode = available / 3;
                if decode == 0 {
                    return Err(crate::allocation::error(
                        cmd,
                        &format!("no room is left to decode {name} after the {} of sources already loaded", bytes_text(self.retained)),
                    ));
                }
                opts.flat_max_pixel_bytes = u64::try_from(decode).unwrap_or(u64::MAX);
            }
            ImportPreflight::Unbounded => {
                return Err(bad(
                    cmd,
                    format!(
                        "{name}: this format has no memory estimate before decoding, so Photomerge cannot fit it into its budget; use a PSD, TIFF or flat raster file, or open it first and add the open document"
                    ),
                ));
            }
        }
        Ok(opts)
    }

    /// Refuses a TIFF whose default page alone would not fit in `available`; a layered TIFF is
    /// also bounded by the importer's own PSD-style preflight.
    fn admit_tiff_header(&self, name: &str, bytes: &[u8], available: usize, cmd: &str) -> Result<()> {
        let info = photocraft_codecs::tiff_info(bytes).map_err(|e| bad(cmd, format!("{name}: {e}")))?;
        let page = info.default_page().and_then(|i| info.pages.get(i)).ok_or_else(|| bad(cmd, format!("{name}: the TIFF has no readable page")))?;
        let bpp = u64::from(page.samples_per_pixel).saturating_mul(u64::from(page.bits_per_sample).div_ceil(8));
        let needed = u64::from(page.width).saturating_mul(u64::from(page.height)).saturating_mul(bpp);
        let available = u64::try_from(available).unwrap_or(u64::MAX);
        if needed > available {
            let needed = usize::try_from(needed).unwrap_or(usize::MAX);
            let available = usize::try_from(available).unwrap_or(usize::MAX);
            return Err(crate::allocation::error(
                cmd,
                &format!(
                    "{name} needs about {} for its {}×{} page, which exceeds the {} left",
                    bytes_text(needed),
                    page.width,
                    page.height,
                    bytes_text(available)
                ),
            ));
        }
        Ok(())
    }
}

/// Sources collected so far, with the common format and the first source's ICC profile.
#[derive(Default)]
struct Sources {
    out: Vec<Source>,
    fmt: Option<PixelFormat>,
    icc: Option<std::sync::Arc<Vec<u8>>>,
}

impl Sources {
    /// Flattens `d` into the common format, admitted by `load` first.
    fn add(&mut self, load: &mut SourceLoad, name: String, d: &Document, encoded: usize, cmd: &str) -> Result<()> {
        let source_fmt = d.pixel_format();
        // Registration and blending work in RGB, so the result is RGB.
        let output_fmt = *self.fmt.get_or_insert_with(|| {
            if matches!(source_fmt.mode, ColorMode::Cmyk | ColorMode::Lab | ColorMode::Multichannel) {
                PixelFormat::new(ColorMode::Rgb, source_fmt.sample, true)
            } else {
                source_fmt
            }
        });
        if self.out.is_empty() && source_fmt.mode == output_fmt.mode {
            self.icc = d.icc_profile.clone();
        }
        let info = d.metadata.exif.as_ref().map(|e| exif::read(e)).unwrap_or_default();
        load.admit(cmd, d, output_fmt.bytes_per_pixel(), encoded)?;
        self.out.push(Source { name, surf: flattened_checked(d, output_fmt, cmd)?, w: d.size.width as usize, h: d.size.height as usize, exif: info });
        Ok(())
    }
}

/// Flattens the open documents and files named by `p` into common-format sources. With a
/// `budget`, each source is read, imported and admitted by [`SourceLoad`]; `None` (Merge to HDR
/// Pro) only flattens, as before. `cmd` names the command in errors.
fn load_sources(s: &Session, p: &Value, cmd: &str, budget: Option<usize>) -> Result<Loaded> {
    let mut src = Sources::default();
    let mut load = SourceLoad::new(budget);
    if p.get("useOpenDocuments").and_then(Value::as_bool).unwrap_or(false) {
        for d in s.documents() {
            src.add(&mut load, d.doc.name.clone(), &d.doc, 0, cmd)?;
        }
    }
    let paths: Vec<String> = match p.get("paths").or_else(|| p.get("input")) {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        Some(Value::String(dir)) => list_images(dir)?,
        _ => Vec::new(),
    };
    for path in &paths {
        let (name, doc, encoded) = load.load_file(s, path, cmd)?;
        src.add(&mut load, name, &doc, encoded, cmd)?;
    }
    if src.out.is_empty() {
        return Err(bad(cmd, "pass \"paths\" (files or a folder) or \"useOpenDocuments\": true"));
    }
    let Some(fmt) = src.fmt else { return Err(bad(cmd, "no readable image sources")) };
    Ok((src.out, fmt, src.icc))
}

/// Luminance and coverage of `surf` over `area`, box-downsampled by `k`.
pub(crate) fn luma_alpha(surf: &Surface, area: Rect, k: usize) -> (usize, usize, Vec<f32>, Vec<f32>) {
    let fmt = surf.format();
    let n = fmt.channels();
    let (w, h) = (area.width() as usize, area.height() as usize);
    let (sw, sh) = (w.div_ceil(k).max(1), h.div_ceil(k).max(1));
    let (mut l, mut a, mut cnt) = (vec![0.0f32; sw * sh], vec![0.0f32; sw * sh], vec![0.0f32; sw * sh]);
    // Row by row keeps memory small for large images.
    for y in 0..h {
        let row = surf.read_region(Rect::new(area.x0, area.y0 + y as i32, area.x1, area.y0 + y as i32 + 1));
        for x in 0..w {
            let p = to_rgba(&fmt, &row[x * n..(x + 1) * n]);
            let i = (y / k) * sw + x / k;
            l[i] += (0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]) * p[3] + (1.0 - p[3]) * 0.0;
            a[i] += p[3];
            cnt[i] += 1.0;
        }
    }
    for i in 0..sw * sh {
        let c = cnt[i].max(1.0);
        l[i] = if a[i] > 0.0 { l[i] / a[i] } else { 0.0 };
        a[i] /= c;
    }
    (sw, sh, l, a)
}

/// Registers `images` (surface, content area) and returns full-resolution placements.
/// `focal35` is the 35 mm-equivalent focal length when known (EXIF).
pub(crate) fn register(images: &[(&Surface, Rect)], layout: Layout, reference: Option<usize>, geometric: bool, focal35: Option<f64>) -> Option<Alignment> {
    let side = images.iter().map(|(_, r)| r.width().max(r.height()) as usize).max().unwrap_or(1);
    let k = side.div_ceil(REGISTER_SIDE).max(1);
    let prepared: Vec<panorama::PreparedImage> = images
        .iter()
        .map(|(s, r)| {
            let (w, h, l, a) = luma_alpha(s, *r, k);
            let valid = a.iter().any(|v| *v < 0.5).then(|| a.iter().map(|v| *v > 0.5).collect());
            (w, h, l, valid)
        })
        .collect();
    let feats = panorama::detect_many(&prepared, 800);
    let matches = panorama::match_pairs(&feats);
    let focal = focal35.map(|f| f / 36.0 * side as f64 / k as f64);
    let a = panorama::align(&feats, &matches, &AlignOptions { layout, focal, reference, geometric })?;
    let kf = k as f64;
    Some(Alignment { placements: a.placements.iter().map(|p| p.as_ref().map(|p| p.scaled(kf))).collect(), focal: a.focal * kf, rms: a.rms * kf, ..a })
}

/// The intensity registration searches planes at most this many pixels across: the pose is
/// resolution independent, so the fit still applies at full resolution.
pub(crate) const DIRECT_SIDE: usize = 2048;

/// A layer and its neighbour must share at least this fraction of the plane at the pose found.
const DIRECT_MIN_OVERLAP: f64 = 0.25;
/// The reference and the warped layer must correlate at least this much: a bracket of one scene
/// correlates near 1 whatever the exposure; unrelated pictures, or a pose that ran into the
/// search box, do not.
const DIRECT_MIN_CORRELATION: f64 = 0.5;

/// The error of either registration when the layers have no detail in common.
pub(crate) const NO_DETAIL: &str = "Auto-Align couldn't find enough matching detail between the layers";

/// Registers `images` (surface, content area, all of one size) by intensity
/// ([`photocraft_algo::direct`]), for layers that nearly coincide (a focus or exposure bracket,
/// a burst): outward from the reference, each layer to the previous one aligned, starting from
/// its pose. The luminance planes are built at the search scale (≤ [`DIRECT_SIDE`] across), so
/// nothing full-resolution is held or warped. A layer whose best pose overlaps its neighbour
/// too little or explains too little of it is no fit, and fails with [`NO_DETAIL`]. Planar
/// layouts only; Auto means a similarity (Collage).
pub(crate) fn register_direct(images: &[(&Surface, Rect)], layout: Layout, reference: usize) -> std::result::Result<Alignment, String> {
    use photocraft_algo::direct::{self, Plane, Pose};
    use photocraft_algo::transform::Homography;
    let (motion, layout) = match layout {
        Layout::Reposition => (panorama::Motion::Translation, Layout::Reposition),
        Layout::Auto | Layout::Collage => (panorama::Motion::Similarity, Layout::Collage),
        Layout::Perspective => (panorama::Motion::Homography, Layout::Perspective),
        Layout::Cylindrical | Layout::Spherical => {
            return Err("the intensity registration supports the auto, perspective, collage and reposition projections".into());
        }
    };
    let n = images.len();
    let Some(&(_, area)) = images.get(reference) else { return Err("no layers to align".into()) };
    let (fw, fh) = (area.width().max(1) as usize, area.height().max(1) as usize);
    if images.iter().any(|(_, r)| (r.width(), r.height()) != (area.width(), area.height())) {
        return Err("the layers to align have different sizes".into());
    }
    // Luminance at the search scale: the box average over k×k pixels, read row by row.
    let k = fw.max(fh).div_ceil(DIRECT_SIDE).max(1);
    let planes: Vec<(Vec<f32>, Option<Vec<bool>>)> = images
        .iter()
        .map(|(s, r)| {
            let (_, _, l, a) = luma_alpha(s, *r, k);
            let valid = a.iter().any(|v| *v < 0.5).then(|| a.iter().map(|v| *v > 0.5).collect());
            (l, valid)
        })
        .collect();
    let (w, h) = (fw.div_ceil(k).max(1), fh.div_ceil(k).max(1));
    let min_overlap = (DIRECT_MIN_OVERLAP * (w * h) as f64) as usize;
    let mut poses: Vec<Option<Pose>> = vec![None; n];
    let mut rmss = Vec::new();
    let mut pairs = Vec::new();
    for dir in [1i64, -1] {
        let Some((ref_luma, ref_valid)) = planes.get(reference) else { continue };
        let (mut prev_luma, mut prev_valid) = (ref_luma.clone(), ref_valid.clone());
        let mut init = Pose::IDENTITY;
        let mut i = reference as i64 + dir;
        while i >= 0 && (i as usize) < n {
            let idx = i as usize;
            let Some((l, v)) = planes.get(idx) else { break };
            let target = Plane { w, h, luma: l, valid: v.as_deref() };
            let reg = direct::register(&Plane { w, h, luma: &prev_luma, valid: prev_valid.as_deref() }, &target, motion, init, 0);
            if reg.overlap < min_overlap || reg.correlation < DIRECT_MIN_CORRELATION {
                return Err(NO_DETAIL.into());
            }
            init = reg.pose;
            (prev_luma, prev_valid) = {
                let (wl, wv) = direct::warp(&target, &reg.pose);
                (wl, Some(wv))
            };
            pairs.push(((i - dir) as usize, idx, reg.overlap * k * k));
            rmss.push(reg.rms);
            poses[idx] = Some(reg.pose);
            i += dir;
        }
    }
    let center = [fw as f64 / 2.0, fh as f64 / 2.0];
    let to_center = Homography([1.0, 0.0, center[0], 0.0, 1.0, center[1], 0.0, 0.0, 1.0]);
    let placement = |hm: &Homography| Placement { projection: panorama::Projection::Plane, focal: fw.max(fh) as f64, center, k1: 0.0, h: hm.mul(&to_center) };
    // The pose is in fractions of the layer: its matrix at full resolution places the layer.
    let placements: Vec<Option<Placement>> =
        (0..n).map(|i| if i == reference { Some(placement(&Homography::IDENTITY)) } else { poses[i].map(|p| placement(&p.matrix(fw, fh))) }).collect();
    let rms = if rmss.is_empty() { 0.0 } else { rmss.iter().sum::<f64>() / rmss.len() as f64 };
    Ok(Alignment { layout, reference, placements, focal: 0.0, k1: 0.0, pairs, rms })
}

/// The 35 mm focal length agreed by the sources' EXIF (median), if any.
fn exif_focal(srcs: &[Source]) -> Option<f64> {
    let mut f: Vec<f64> = srcs.iter().filter_map(|s| s.exif.focal_length_35mm).collect();
    f.sort_by(f64::total_cmp);
    f.get(f.len() / 2).copied()
}

/// Warps the content of `surf` inside `rect` by a placement shifted by `offset`; image
/// coordinates are surface coordinates minus `origin`.
pub(crate) fn warp_placed(surf: &Surface, rect: Rect, origin: (f64, f64), pl: &Placement, offset: (f64, f64), interp: Interp) -> Result<Surface> {
    let pl = pl.translated(offset.0, offset.1);
    if let Some(h) = plane_matrix(&pl, origin) {
        return Ok(warp_surface(surf, rect, &h, interp)?);
    }
    Ok(warp_mesh_surface(surf, rect, &|x, y| pl.forward(x - origin.0, y - origin.1), interp))
}

/// The projective matrix of a planar, distortion-free placement (surface px → panorama px).
pub(crate) fn plane_matrix(pl: &Placement, origin: (f64, f64)) -> Option<Homography> {
    (pl.projection == panorama::Projection::Plane && pl.k1 == 0.0).then(|| {
        let t = Homography([1.0, 0.0, -pl.center[0] - origin.0, 0.0, 1.0, -pl.center[1] - origin.1, 0.0, 0.0, 1.0]);
        pl.h.mul(&t)
    })
}

/// Photomerge's working-memory budget in bytes, from [`budget_from`]: the `memoryBudget` param,
/// else `PHOTOCRAFT_PHOTOMERGE_BUDGET_MB`, else half of the machine's physical memory.
fn photomerge_memory_budget(p: &Value) -> usize {
    let env_mb = std::env::var("PHOTOCRAFT_PHOTOMERGE_BUDGET_MB").ok();
    budget_from(p.get("memoryBudget").and_then(Value::as_u64), env_mb.as_deref(), crate::memory::physical_memory())
}

/// The budget: `param` (bytes) wins, then `env_mb` (a budget in MiB), then half of `ram` bytes
/// (4 GiB assumed when unknown) but at least 1 GiB.
fn budget_from(param: Option<u64>, env_mb: Option<&str>, ram: Option<u64>) -> usize {
    if let Some(b) = param {
        return usize::try_from(b).unwrap_or(usize::MAX);
    }
    if let Some(mb) = env_mb.and_then(|v| v.trim().parse::<u64>().ok()) {
        return usize::try_from(mb.saturating_mul(1 << 20)).unwrap_or(usize::MAX);
    }
    let ram = ram.unwrap_or(4 << 30);
    usize::try_from((ram / 2).max(1 << 30)).unwrap_or(usize::MAX)
}

/// Estimated peak working memory of a Photomerge and the part of it that stays allocated while
/// bands are built (the ROIs, weights and accumulators; what is left of the budget goes to the
/// bands, see [`panorama::band_bytes_bound`]). `source_sizes` and `placed_sizes` are the source
/// and warped sizes of the placed images, `cw` × `ch` the canvas, `cc` its colour channels and
/// `bpp` its bytes per pixel.
///
/// The peak is the largest of the phases, since each frees what the next no longer needs: warping
/// (sources and warped images), ROI conversion (ROIs, weights, warped images), the bands (ROIs,
/// accumulators and the largest band), the collapse (ROIs, accumulators and the running result),
/// the output (ROIs, blended colour, composite and output layers) and the fill (layers, composite
/// and the fill's own planes).
#[allow(clippy::too_many_arguments)]
fn estimate_photomerge_memory(
    source_sizes: &[(usize, usize)],
    placed_sizes: &[(usize, usize)],
    cw: usize,
    ch: usize,
    cc: usize,
    bpp: usize,
    blend: bool,
    fill: bool,
) -> (usize, usize) {
    let px_sum = |sizes: &[(usize, usize)]| sizes.iter().map(|&(w, h)| w.saturating_mul(h)).fold(0usize, usize::saturating_add);
    let sources = px_sum(source_sizes).saturating_mul(bpp);
    let roi_px = px_sum(placed_sizes);
    let warped = roi_px.saturating_mul(bpp);
    if !blend {
        return (sources.saturating_add(warped).max(warped), 0);
    }
    // ROI colours, alpha and the seam weights (f32 each).
    let rois_weights = roi_px.saturating_mul((cc + 2) * 4);
    let canvas_px = cw.saturating_mul(ch);
    // The accumulator and weight-sum pyramids (4/3 of a plane each).
    let accumulators = canvas_px.saturating_mul((cc + 1) * 4 * 4 / 3);
    let blend_out = canvas_px.saturating_mul((cc + 1) * 4);
    let output_layers = roi_px.saturating_mul(bpp + 1);
    let composite = canvas_px.saturating_mul(bpp);
    let fill_bytes = if fill { composite.saturating_add(24 << 20) } else { 0 };

    // The levels `seam_blend` blends with.
    let levels = panorama::blend_levels(cw, ch).min(7);
    let max_band_cost = placed_sizes.iter().map(|&(w, h)| panorama::band_bytes_bound(w, h, cc, levels, cw, ch)).max().unwrap_or(0);

    let fixed_during_bands = rois_weights.saturating_add(accumulators);
    let phases = [
        sources.saturating_add(warped),
        rois_weights.saturating_add(warped),
        fixed_during_bands.saturating_add(max_band_cost),
        fixed_during_bands.saturating_add(canvas_px.saturating_mul(cc * 4)),
        rois_weights.saturating_add(blend_out).saturating_add(composite).saturating_add(output_layers),
        output_layers.saturating_add(composite).saturating_add(fill_bytes),
    ];
    (phases.into_iter().max().unwrap_or(0), fixed_during_bands)
}

/// Splits a surface region into colour channels and coverage.
fn roi_of(surf: &Surface, area: Rect) -> Result<RoiImage> {
    let fmt = surf.format();
    let n = fmt.channels();
    let cc = fmt.mode.color_channels();
    let (w, h) = (area.width() as usize, area.height() as usize);
    let total_px = w.checked_mul(h).and_then(|v| v.checked_mul(cc)).ok_or_else(|| crate::allocation::error("Photomerge", "ROI size overflowed"))?;
    let total_alpha = w.checked_mul(h).ok_or_else(|| crate::allocation::error("Photomerge", "ROI size overflowed"))?;
    let mut px = crate::allocation::capacity(total_px, "Photomerge ROI")?;
    let mut alpha = crate::allocation::capacity(total_alpha, "Photomerge alpha")?;
    if w > 0 && h > 0 {
        let mut row = vec![0.0f32; w * n];
        for y in 0..h {
            let row_rect = Rect::new(area.x0, area.y0 + y as i32, area.x1, area.y0 + y as i32 + 1);
            surf.read_region_into(row_rect, &mut row);
            for q in row.chunks_exact(n) {
                px.extend_from_slice(&q[..cc]);
                alpha.push(if fmt.alpha { q[n - 1] } else { 1.0 });
            }
        }
    }
    Ok(RoiImage { x0: area.x0, y0: area.y0, w, h, ch: cc, px, alpha })
}

/// Placements, source sizes and the panorama offset (for vignette removal).
pub(crate) type BlendGeometry<'a> = (&'a [Placement], &'a [(usize, usize)], (f64, f64));

/// Result of [`seam_blend`]: per input, the pixels to keep (blended inside its seam region)
/// and its mask; plus the blended composite.
pub(crate) struct Blended {
    pub layers: Vec<(Surface, Surface)>,
    pub composite: Surface,
    pub gains: Vec<Vec<f32>>,
    pub vignette: [f64; 2],
}

/// How [`seam_blend`] blends: `seamless` blends the bands (else hard seams), `gains` compensates
/// exposure and colour, `geometry` enables vignette removal. `budget` is the bytes of bands built
/// at a time and the cap on any single band (an error beyond it); `None` is 5 GiB batches with
/// no cap.
pub(crate) struct BlendOptions<'a> {
    pub seamless: bool,
    pub gains: bool,
    pub geometry: Option<BlendGeometry<'a>>,
    pub budget: Option<usize>,
}

/// Applies the photometric model (gains, vignetting) to `rois`; returns the gains and the vignette.
fn apply_photometric(rois: &mut [RoiImage], fmt: PixelFormat, geometry: Option<BlendGeometry<'_>>, seamless: bool, gains: bool) -> (Vec<Vec<f32>>, [f64; 2]) {
    // Photometric model in panorama coordinates relative to the canvas origin used by placements.
    let shifted: Option<Vec<Placement>> = geometry.map(|(pl, _, off)| pl.iter().map(|p| p.translated(off.0, off.1)).collect());
    let geo = match (&shifted, geometry) {
        (Some(pl), Some((_, sizes, _))) => Some((pl.as_slice(), sizes)),
        _ => None,
    };
    let photo = panorama::photometric(rois, geo, seamless && gains, geo.is_some());
    let vig = photo.vignette != [0.0, 0.0];
    for (i, roi) in rois.iter_mut().enumerate() {
        let ch = roi.ch;
        if photo.gains[i].iter().all(|g| (*g - 1.0).abs() < 1e-6) && !vig {
            continue;
        }
        for y in 0..roi.h {
            for x in 0..roi.w {
                let r2 = if vig && let Some((pl, sizes)) = geo {
                    pl[i].inverse((roi.x0 + x as i32) as f64 + 0.5, (roi.y0 + y as i32) as f64 + 0.5).map_or(0.0, |(sx, sy)| {
                        let (w, h) = (sizes[i].0 as f64, sizes[i].1 as f64);
                        let (dx, dy) = (sx - w / 2.0, sy - h / 2.0);
                        (dx * dx + dy * dy) / (w * w / 4.0 + h * h / 4.0)
                    })
                } else {
                    0.0
                };
                let k = y * roi.w + x;
                for c in 0..ch {
                    let f = photo.factor(i, c, r2);
                    let v = &mut roi.px[k * ch + c];
                    *v = if fmt.mode == ColorMode::Cmyk { 1.0 - (1.0 - *v) * f } else { *v * f };
                    if fmt.sample != SampleType::F32 {
                        *v = v.clamp(0.0, 1.0);
                    }
                }
            }
        }
    }
    (photo.gains, photo.vignette)
}

/// Per-image seam weights (1 inside an image's seam region, else 0) over each ROI.
fn compute_seam_weights(rois: &[RoiImage], canvas: Rect, order: &[usize]) -> Vec<Vec<f32>> {
    let n = rois.len();
    // Seams on a ≤ 1000 px grid.
    let (cw, chh) = (canvas.width().max(1) as usize, canvas.height().max(1) as usize);
    let s = cw.max(chh).div_ceil(1000).max(1);
    let (gw, gh) = (cw.div_ceil(s), chh.div_ceil(s));
    let mut luma = vec![vec![0.0f32; gw * gh]; n];
    let mut cover = vec![vec![false; gw * gh]; n];
    for (i, roi) in rois.iter().enumerate() {
        let mut acc = vec![(0.0f32, 0.0f32, 0u32); gw * gh];
        for y in 0..roi.h {
            for x in 0..roi.w {
                let (u, v) = ((roi.x0 - canvas.x0) as usize + x, (roi.y0 - canvas.y0) as usize + y);
                let g = (v / s).min(gh - 1) * gw + (u / s).min(gw - 1);
                let k = y * roi.w + x;
                let l = roi.px[k * roi.ch..(k + 1) * roi.ch].iter().sum::<f32>() / roi.ch as f32;
                acc[g].0 += l;
                acc[g].1 += roi.alpha[k];
                acc[g].2 += 1;
            }
        }
        for g in 0..gw * gh {
            let (l, a, c) = acc[g];
            if c > 0 {
                luma[i][g] = l / c as f32;
                cover[i][g] = a / (s * s) as f32 > 0.98;
            }
        }
    }
    let labels = panorama::seam_labels(gw, gh, &luma, &cover, order);
    // Full-resolution weights: the seam label, falling back to the earliest covering image in
    // `order` where the label's image doesn't cover the pixel.
    let rank: Vec<usize> = {
        let mut r = vec![usize::MAX; n];
        for (k, &i) in order.iter().enumerate() {
            r[i] = k;
        }
        r
    };
    let alpha_at = |j: usize, x: i32, y: i32| -> f32 {
        let r = &rois[j];
        let (lx, ly) = (x - r.x0, y - r.y0);
        if lx < 0 || ly < 0 || lx as usize >= r.w || ly as usize >= r.h { 0.0 } else { r.alpha[ly as usize * r.w + lx as usize] }
    };
    rois.iter()
        .enumerate()
        .map(|(i, roi)| {
            (0..roi.w * roi.h)
                .map(|k| {
                    let (x, y) = (roi.x0 + (k % roi.w) as i32, roi.y0 + (k / roi.w) as i32);
                    if roi.alpha[k] < 0.5 {
                        return 0.0;
                    }
                    let (u, v) = ((x - canvas.x0) as usize, (y - canvas.y0) as usize);
                    let lab = labels[(v / s).min(gh - 1) * gw + (u / s).min(gw - 1)];
                    if lab == i as i32 {
                        return 1.0;
                    }
                    if lab >= 0 && alpha_at(lab as usize, x, y) >= 0.5 {
                        return 0.0;
                    }
                    // Fallback: the earliest covering image wins.
                    let first = (0..n).filter(|&j| alpha_at(j, x, y) >= 0.5).min_by_key(|&j| rank[j]);
                    if first == Some(i) { 1.0 } else { 0.0 }
                })
                .collect()
        })
        .collect()
}

/// The blended composite over `canvas`, its alpha the coverage.
fn build_composite(canvas: Rect, fmt: PixelFormat, blend: &[f32], cov: &[f32]) -> Surface {
    let (cw, chh) = (canvas.width().max(1) as usize, canvas.height().max(1) as usize);
    let cc = fmt.mode.color_channels();
    let nch = fmt.channels();
    let mut composite = Surface::new(fmt);
    if cw > 0 && chh > 0 {
        let mut row_comp = vec![0.0f32; cw * nch];
        for y in 0..chh {
            for x in 0..cw {
                let i = y * cw + x;
                for c in 0..cc {
                    let v = blend[i * cc + c];
                    row_comp[x * nch + c] = if fmt.sample == SampleType::F32 { v } else { v.clamp(0.0, 1.0) };
                }
                row_comp[x * nch + nch - 1] = cov[i];
            }
            let row_area = Rect::new(canvas.x0, canvas.y0 + y as i32, canvas.x1, canvas.y0 + y as i32 + 1);
            composite.write_region(row_area, &row_comp);
        }
        composite.prune();
    }
    composite
}

/// Per input, its pixels (blended inside its seam region when `seamless`) and its seam mask.
fn build_layers(
    rois: &[RoiImage],
    weights: &[Vec<f32>],
    canvas: Rect,
    fmt: PixelFormat,
    blend: &[f32],
    seamless: bool,
    check_cancel: Option<&dyn Fn() -> Result<()>>,
) -> Result<Vec<(Surface, Surface)>> {
    let cw = canvas.width().max(1) as usize;
    let cc = fmt.mode.color_channels();
    let nch = fmt.channels();
    let mut layers = Vec::with_capacity(rois.len());
    for (roi, wt) in rois.iter().zip(weights) {
        if let Some(check) = check_cancel {
            check()?;
        }
        let area = Rect::new(roi.x0, roi.y0, roi.x0 + roi.w as i32, roi.y0 + roi.h as i32);
        let mut ls = Surface::new(fmt);
        let mut ms = Surface::with_default(PixelFormat::GRAY8, &[0.0]);
        if !area.is_empty() && roi.w > 0 && roi.h > 0 {
            let mut row_px = vec![0.0f32; roi.w * nch];
            let mut row_mask = vec![0.0f32; roi.w];
            for y in 0..roi.h {
                let v = (roi.y0 - canvas.y0) as usize + y;
                for x in 0..roi.w {
                    let u = (roi.x0 - canvas.x0) as usize + x;
                    let k = y * roi.w + x;
                    let g = v * cw + u;
                    let inside = wt[k] > 0.5;
                    for c in 0..cc {
                        row_px[x * nch + c] = if inside && seamless { data_clamp(blend[g * cc + c], fmt) } else { roi.px[k * cc + c] };
                    }
                    row_px[x * nch + nch - 1] = roi.alpha[k];
                    row_mask[x] = wt[k];
                }
                let row_area = Rect::new(roi.x0, roi.y0 + y as i32, roi.x0 + roi.w as i32, roi.y0 + y as i32 + 1);
                ls.write_region(row_area, &row_px);
                ms.write_region(row_area, &row_mask);
            }
        }
        ls.prune();
        ms.prune();
        layers.push((ls, ms));
    }
    Ok(layers)
}

/// Gain compensation (+ optional vignetting), seams in `order`, multi-band blending. Inputs are
/// surfaces already in panorama coordinates with alpha. `geometry` enables vignette removal.
pub(crate) fn seam_blend(
    warped: Vec<Surface>,
    canvas: Rect,
    order: &[usize],
    opt: BlendOptions<'_>,
    check_cancel: Option<&dyn Fn() -> Result<()>>,
) -> Result<Blended> {
    let n = warped.len();
    let fmt = warped.first().map_or(PixelFormat::RGBA8, Surface::format);
    let mut rois: Vec<RoiImage> = Vec::with_capacity(n);
    for s in warped {
        if let Some(check) = check_cancel {
            check()?;
        }
        let b = s.content_bounds().intersect(&canvas);
        if b.is_empty() {
            rois.push(RoiImage { x0: 0, y0: 0, w: 0, h: 0, ch: fmt.mode.color_channels(), px: Vec::new(), alpha: Vec::new() });
        } else {
            rois.push(roi_of(&s, b)?);
        }
    }
    let (gains, vignette) = apply_photometric(&mut rois, fmt, opt.geometry, opt.seamless, opt.gains);
    let weights = compute_seam_weights(&rois, canvas, order);
    // Shift ROIs to canvas-relative coordinates for the blend, and back after: a shifted copy
    // would hold every source's pixels twice (#7758).
    for r in &mut rois {
        r.x0 -= canvas.x0;
        r.y0 -= canvas.y0;
    }
    if let Some(check) = check_cancel {
        check()?;
    }
    let (cw, chh) = (canvas.width().max(1) as usize, canvas.height().max(1) as usize);
    let levels = if opt.seamless { panorama::blend_levels(cw, chh).min(7) } else { 1 };
    let band_budget = opt.budget.unwrap_or(panorama::BAND_BUDGET);
    let (blend, cov) = panorama::try_multiband_batched(cw, chh, &rois, &weights, levels, band_budget, opt.budget)?;
    for r in &mut rois {
        r.x0 += canvas.x0;
        r.y0 += canvas.y0;
    }
    if let Some(check) = check_cancel {
        check()?;
    }
    let composite = build_composite(canvas, fmt, &blend, &cov);
    let layers = build_layers(&rois, &weights, canvas, fmt, &blend, opt.seamless, check_cancel)?;
    Ok(Blended { layers, composite, gains, vignette })
}

fn data_clamp(v: f32, fmt: PixelFormat) -> f32 {
    if fmt.sample == SampleType::F32 { v } else { v.clamp(0.0, 1.0) }
}

/// Seam order: the reference first, then by distance of each image's centre from it.
pub(crate) fn seam_order(warped: &[Surface], reference: usize) -> Vec<usize> {
    let centre = |s: &Surface| {
        let b = s.content_bounds();
        ((b.x0 + b.x1) as f64 / 2.0, (b.y0 + b.y1) as f64 / 2.0)
    };
    let rc = centre(&warped[reference.min(warped.len() - 1)]);
    let mut order: Vec<usize> = (0..warped.len()).collect();
    order.sort_by(|&a, &b| {
        let (ca, cb) = (centre(&warped[a]), centre(&warped[b]));
        let (da, db) = ((ca.0 - rc.0).hypot(ca.1 - rc.1), (cb.0 - rc.0).hypot(cb.1 - rc.1));
        (a != reference).cmp(&(b != reference)).then(da.total_cmp(&db))
    });
    order
}

// ---------- Photomerge ----------

fn photomerge(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.photomerge";
    let t0 = Stopwatch::start();
    let layout_s = p.get("layout").and_then(Value::as_str).unwrap_or("auto");
    let layout =
        Layout::parse(layout_s).ok_or_else(|| bad(cmd, format!("unknown layout `{layout_s}` (auto|perspective|cylindrical|spherical|collage|reposition)")))?;
    let blend = p.get("blend").and_then(Value::as_bool).unwrap_or(true);
    let vignette = p.get("vignetteRemoval").and_then(Value::as_bool).unwrap_or(false);
    let geometric = p.get("geometricCorrection").and_then(Value::as_bool).unwrap_or(false);
    let fill = p.get("contentAwareFill").and_then(Value::as_bool).unwrap_or(false);
    // Resolved before loading: the sources are flattened and retained one at a time, so the
    // budget has to bound them before the first surface is allocated, not after they all exist.
    let budget = photomerge_memory_budget(p);
    let (mut srcs, fmt, icc) = load_sources(s, p, cmd, Some(budget))?;
    if srcs.len() < 2 {
        return Err(bad(cmd, "Photomerge needs two or more images"));
    }
    let focal_param = p.get("focalLength").and_then(Value::as_f64).filter(|f| *f > 0.0);
    let doc_name = format!("Untitled_Panorama{}", s.documents().len() + 1);
    // A background job when started with `Session::start` (#210): alignment, warping and
    // blending run on a worker, checking for cancellation between stages and images.
    crate::jobs::run(
        s,
        "Photomerge",
        false,
        move |ctx| {
            ctx.progress(0.0, "Photomerge: aligning images");
            let focal35 = focal_param.or_else(|| exif_focal(&srcs));
            let images: Vec<(&Surface, Rect)> = srcs.iter().map(|s| (&s.surf, Rect::new(0, 0, s.w as i32, s.h as i32))).collect();
            let al = register(&images, layout, None, geometric, focal35)
                .ok_or_else(|| EngineError::Other("Photomerge couldn't find enough matching detail between the images".into()))?;
            let t_reg = t0.ms() / 1000.0;
            ctx.check()?;
            ctx.progress(0.35, "Photomerge: warping images");
            let placed: Vec<usize> = (0..srcs.len()).filter(|&i| al.placements.get(i).is_some_and(Option::is_some)).collect();
            // Parallel to `placed`.
            let placements: Vec<&Placement> = placed.iter().filter_map(|&i| al.placements.get(i).and_then(Option::as_ref)).collect();
            let failed: Vec<String> = (0..srcs.len()).filter(|&i| al.placements.get(i).is_none_or(Option::is_none)).map(|i| srcs[i].name.clone()).collect();
            // Canvas: union of the placed images.
            let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
            // Each placed image's warped extent (the bounds of its placement).
            let mut extents: Vec<(f64, f64)> = Vec::with_capacity(placed.len());
            for (&i, pl) in placed.iter().zip(&placements) {
                let b = pl.bounds(srcs[i].w as f64, srcs[i].h as f64);
                extents.push((b[2] - b[0], b[3] - b[1]));
                x0 = x0.min(b[0]);
                y0 = y0.min(b[1]);
                x1 = x1.max(b[2]);
                y1 = y1.max(b[3]);
            }
            let (cw, ch) = ((x1 - x0).ceil(), (y1 - y0).ceil());
            if !(cw.is_finite() && ch.is_finite()) || cw <= 0.0 || ch <= 0.0 || cw > 30_000.0 || ch > 30_000.0 || cw * ch > 6.0e8 {
                return Err(EngineError::Other(format!("the panorama would be {cw}×{ch} px; try another layout")));
            }
            let canvas = Rect::new(0, 0, cw as i32, ch as i32);
            let offset = (-x0.floor(), -y0.floor());
            let sizes: Vec<(usize, usize)> = placed.iter().map(|&i| (srcs[i].w, srcs[i].h)).collect();
            let cc = fmt.mode.color_channels();
            let bpp = fmt.bytes_per_pixel();
            // The estimate follows the warped images (one pixel of slack for sub-pixel bounds), clipped to the canvas.
            let warped_sizes: Vec<(usize, usize)> =
                extents.iter().map(|&(w, h)| ((w.ceil() + 1.0).max(0.0).min(cw) as usize, (h.ceil() + 1.0).max(0.0).min(ch) as usize)).collect();
            let (est_needed, fixed_during_bands) = estimate_photomerge_memory(&sizes, &warped_sizes, cw as usize, ch as usize, cc, bpp, blend, fill);
            if est_needed > budget {
                let mb_est = est_needed / (1 << 20);
                let mb_budget = budget / (1 << 20);
                return Err(crate::allocation::error(
                    "Photomerge",
                    &format!("estimated memory of {mb_est} MiB exceeds budget of {mb_budget} MiB; use fewer or smaller images, or raise \"memoryBudget\""),
                ));
            }
            for (i, s) in srcs.iter_mut().enumerate() {
                if al.placements.get(i).is_none_or(Option::is_none) {
                    s.surf = Surface::new(fmt);
                }
            }
            let mut warped: Vec<Surface> = Vec::with_capacity(placed.len());
            for (k, (&i, pl)) in placed.iter().zip(&placements).enumerate() {
                ctx.check()?;
                ctx.progress(0.35 + 0.25 * k as f32 / placed.len().max(1) as f32, "");
                let src_rect = Rect::new(0, 0, srcs[i].w as i32, srcs[i].h as i32);
                let w = warp_placed(&srcs[i].surf, src_rect, (0.0, 0.0), pl, offset, Interp::Bicubic)?;
                srcs[i].surf = Surface::new(fmt);
                warped.push(w);
            }
            let t_warp = t0.ms() / 1000.0;
            ctx.check()?;
            ctx.progress(0.6, "Photomerge: blending images");
            let mut doc = Document::new(doc_name, Size::new(cw as u32, ch as u32), fmt.mode, fmt.sample);
            doc.icc_profile = icc;
            let mut info = json!({});
            let ref_pos = placed.iter().position(|&i| i == al.reference).unwrap_or(0);
            if blend {
                let pls: Vec<Placement> = placements.iter().map(|p| (*p).clone()).collect();
                let order = seam_order(&warped, ref_pos);
                let geo = vignette.then_some((pls.as_slice(), sizes.as_slice(), offset));
                let remaining_band_budget = budget.saturating_sub(fixed_during_bands);
                let check_cancel = || ctx.check();
                let opt = BlendOptions { seamless: true, gains: true, geometry: geo, budget: Some(remaining_band_budget) };
                let b = seam_blend(warped, canvas, &order, opt, Some(&check_cancel))?;
                for (k, (px, mask)) in b.layers.into_iter().enumerate() {
                    let mut l = Layer::new(srcs[placed[k]].name.clone(), LayerContent::Raster(px));
                    let mut m = LayerMask::hide_all();
                    m.surface = mask;
                    l.mask = Some(m);
                    doc.layers.push(l);
                }
                info = json!({"gains": b.gains, "vignette": b.vignette});
                ctx.check()?;
                if fill {
                    ctx.progress(0.85, "Photomerge: filling edges");
                    let filled = content_aware_fill(&b.composite, canvas);
                    doc.layers.push(Layer::new("Content-Aware Fill", LayerContent::Raster(filled)));
                }
            } else {
                for (k, px) in warped.into_iter().enumerate() {
                    doc.layers.push(Layer::new(srcs[placed[k]].name.clone(), LayerContent::Raster(px)));
                }
            }
            let n = doc.layers.len();
            ctx.check()?;
            ctx.progress(1.0, "");
            Ok((
                doc,
                json!({
                    "document": Value::Null,
                    "layout": al.layout.name(),
                    "reference": srcs[al.reference].name,
                    "focalLength35": al.focal / (srcs.iter().map(|s| s.w.max(s.h)).max().unwrap_or(1) as f64) * 36.0,
                    "geometricK1": al.k1,
                    "rms": al.rms,
                    "pairs": al.pairs,
                    "placed": placed.iter().map(|&i| srcs[i].name.clone()).collect::<Vec<_>>(),
                    "failed": failed,
                    "width": cw as u32,
                    "height": ch as u32,
                    "layers": n,
                    "photometric": info,
                    "ms": {"register": t_reg * 1000.0, "warp": (t_warp - t_reg) * 1000.0, "total": t0.ms()},
                }),
            ))
        },
        |s, (doc, mut info): (Document, Value)| {
            info["document"] = json!(s.add_document(doc, None));
            Ok(info)
        },
    )
}

/// Fills the transparent areas of a composite (inside its bounding rectangle) with
/// content-aware fill, at a reduced scale for large panoramas.
fn content_aware_fill(comp: &Surface, canvas: Rect) -> Surface {
    use photocraft_algo::content_aware::{FillOptions, fill};
    let fmt = comp.format();
    let n = fmt.channels();
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let k = ((w * h) as f64 / 1.5e6).sqrt().ceil().max(1.0) as usize;
    let (sw, sh) = (w.div_ceil(k), h.div_ceil(k));
    let cc = n - 1;
    let mut small = vec![0.0f32; sw * sh * cc];
    let mut hole = vec![false; sw * sh];
    let mut acc_a = vec![0.0f32; sw * sh];
    let mut cnt = vec![0.0f32; sw * sh];
    for y in 0..h {
        let row = comp.read_region(Rect::new(canvas.x0, canvas.y0 + y as i32, canvas.x1, canvas.y0 + y as i32 + 1));
        for x in 0..w {
            let i = (y / k) * sw + x / k;
            let px = &row[x * n..(x + 1) * n];
            let a = px[cc];
            for c in 0..cc {
                small[i * cc + c] += px[c] * a;
            }
            acc_a[i] += a;
            cnt[i] += 1.0;
        }
    }
    for i in 0..sw * sh {
        let a = acc_a[i];
        for c in 0..cc {
            small[i * cc + c] = if a > 0.0 { small[i * cc + c] / a } else { 0.0 };
        }
        hole[i] = a / cnt[i].max(1.0) < 0.99;
    }
    let source: Vec<bool> = hole.iter().map(|h| !h).collect();
    let filled = fill(sw, sh, cc, &small, &hole, &source, &FillOptions::default());
    let mut s = Surface::new(fmt);
    for y in 0..h {
        let area = Rect::new(canvas.x0, canvas.y0 + y as i32, canvas.x1, canvas.y0 + y as i32 + 1);
        let full = comp.read_region(area);
        let mut out = vec![0.0f32; w * n];
        for x in 0..w {
            let o = x * n;
            let a = full[o + cc];
            let i = (y / k) * sw + x / k;
            for c in 0..cc {
                // Bilinear upsampling of the fill under partially covered pixels.
                let fx = ((x as f32 + 0.5) / k as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
                let fy = ((y as f32 + 0.5) / k as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
                let (xi, yi) = (fx as usize, fy as usize);
                let (x2, y2) = ((xi + 1).min(sw - 1), (yi + 1).min(sh - 1));
                let (tx, ty) = (fx - xi as f32, fy - yi as f32);
                let at = |xx: usize, yy: usize| filled[(yy * sw + xx) * cc + c];
                let fv = (at(xi, yi) * (1.0 - tx) + at(x2, yi) * tx) * (1.0 - ty) + (at(xi, y2) * (1.0 - tx) + at(x2, y2) * tx) * ty;
                let _ = i;
                out[o + c] = full[o + c] * a + fv * (1.0 - a);
            }
            out[o + cc] = 1.0;
        }
        s.write_region(area, &out);
    }
    s.prune();
    s
}

// ---------- Merge to HDR Pro ----------

fn shift_rgba(px: &[[f32; 4]], w: usize, h: usize, dx: i32, dy: i32) -> Vec<[f32; 4]> {
    (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as i32 - dx, (i / w) as i32 - dy);
            px[(y.clamp(0, h as i32 - 1) as usize) * w + x.clamp(0, w as i32 - 1) as usize]
        })
        .collect()
}

fn merge_to_hdr(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.mergeToHdrPro";
    let t0 = Stopwatch::start();
    let (srcs, _fmt, icc) = load_sources(s, p, cmd, None)?;
    if srcs.len() < 2 {
        return Err(bad(cmd, "Merge to HDR Pro needs two or more exposures"));
    }
    let (w, h) = (srcs[0].w, srcs[0].h);
    if srcs.iter().any(|s| s.w != w || s.h != h) {
        return Err(EngineError::Other("the exposures must have the same pixel dimensions".into()));
    }
    let area = Rect::new(0, 0, w as i32, h as i32);
    let mut imgs: Vec<Vec<[f32; 4]>> = srcs
        .iter()
        .map(|s| {
            let mut v = vec![[0.0f32; 4]; w * h];
            s.surf.read_rgba_into(area, &mut v);
            v
        })
        .collect();
    // Exposures: explicit EV, EXIF, or estimated.
    let (exposures, source): (Vec<f64>, &str) = match p.get("exposures").and_then(Value::as_array) {
        Some(a) if a.len() == srcs.len() => (a.iter().map(|v| 2f64.powf(v.as_f64().unwrap_or(0.0))).collect(), "params"),
        Some(_) => return Err(bad(cmd, "`exposures` needs one EV value per image")),
        None => match srcs.iter().map(|s| s.exif.exposure_factor()).collect::<Option<Vec<f64>>>() {
            Some(e) => (e, "exif"),
            None => {
                let refs: Vec<&[[f32; 4]]> = imgs.iter().map(Vec::as_slice).collect();
                (hdr::estimate_exposures(&refs), "estimated")
            }
        },
    };
    if exposures.iter().any(|e| !(e.is_finite() && *e > 0.0)) {
        return Err(bad(cmd, "exposures must be positive"));
    }
    // Alignment (Ward's MTB) to the middle exposure, on a downsampled grey copy.
    let mut order: Vec<usize> = (0..srcs.len()).collect();
    order.sort_by(|&a, &b| exposures[a].total_cmp(&exposures[b]));
    let mid = order[order.len() / 2];
    let mut offsets = vec![(0, 0); srcs.len()];
    if p.get("align").and_then(Value::as_bool).unwrap_or(true) {
        let k = w.max(h).div_ceil(1536).max(1);
        let grey = |im: &[[f32; 4]]| -> (usize, usize, Vec<f32>) {
            let (sw, sh) = (w.div_ceil(k), h.div_ceil(k));
            let mut g = vec![0.0f32; sw * sh];
            let mut c = vec![0.0f32; sw * sh];
            for (i, q) in im.iter().enumerate() {
                let j = (i / w / k) * sw + (i % w) / k;
                g[j] += (54.0 * q[0] + 183.0 * q[1] + 19.0 * q[2]) / 256.0;
                c[j] += 1.0;
            }
            (sw, sh, g.iter().zip(&c).map(|(a, b)| a / b.max(1.0)).collect())
        };
        let (sw, sh, gref) = grey(&imgs[mid]);
        for i in 0..srcs.len() {
            if i == mid {
                continue;
            }
            let (_, _, gi) = grey(&imgs[i]);
            let (dx, dy) = hdr::mtb_offset(sw, sh, &gref, &gi, 5);
            offsets[i] = (dx * k as i32, dy * k as i32);
            if offsets[i] != (0, 0) {
                imgs[i] = shift_rgba(&imgs[i], w, h, offsets[i].0, offsets[i].1);
            }
        }
    }
    let ghost_base = p.get("ghostBase").and_then(Value::as_u64).map(|v| v as usize);
    if ghost_base.is_some_and(|g| g >= srcs.len()) {
        return Err(bad(cmd, "`ghostBase` is an image index"));
    }
    let refs: Vec<&[[f32; 4]]> = imgs.iter().map(Vec::as_slice).collect();
    let merged = hdr::merge(
        w,
        h,
        &refs,
        &MergeOptions {
            exposures: exposures.clone(),
            remove_ghosts: p.get("removeGhosts").and_then(Value::as_bool).unwrap_or(false),
            ghost_base,
            response: None,
        },
    );
    let mode = p.get("mode").and_then(|v| v.as_str().map(str::to_string).or_else(|| v.as_u64().map(|n| n.to_string()))).unwrap_or_else(|| "32".into());
    let (depth, mut px) = match mode.as_str() {
        "32" => (SampleType::F32, merged.px.clone()),
        "16" | "8" => {
            let method = p.get("method").and_then(Value::as_str).unwrap_or("localAdaptation");
            let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d) as f32;
            let tm = match method {
                "localAdaptation" => ToneMethod::LocalAdaptation(HdrToning {
                    radius: f("radius", 7.0).clamp(1.0, 500.0),
                    strength: f("strength", 0.52).clamp(0.1, 4.0),
                    gamma: f("gamma", 1.0).clamp(0.1, 2.0),
                    exposure: f("exposure", 0.0).clamp(-5.0, 5.0),
                    detail: f("detail", 30.0).clamp(-100.0, 300.0),
                    shadow: f("shadow", 0.0).clamp(-100.0, 100.0),
                    highlight: f("highlight", 0.0).clamp(-100.0, 100.0),
                    vibrance: f("vibrance", 0.0).clamp(-100.0, 100.0),
                    saturation: f("saturation", 20.0).clamp(-100.0, 100.0),
                    curve: Vec::new(),
                }),
                "exposureGamma" => ToneMethod::ExposureGamma { exposure: f("exposure", 0.0), gamma: f("gamma", 1.0) },
                "highlightCompression" => ToneMethod::HighlightCompression,
                "equalizeHistogram" => ToneMethod::EqualizeHistogram,
                other => return Err(bad(cmd, format!("unknown method `{other}` (localAdaptation|exposureGamma|highlightCompression|equalizeHistogram)"))),
            };
            let mut v = merged.px.clone();
            hdr::tone_map(&mut v, w, h, &tm);
            (if mode == "16" { SampleType::U16 } else { SampleType::U8 }, v)
        }
        other => return Err(bad(cmd, format!("`mode` must be 32|16|8, not {other}"))),
    };
    for q in px.iter_mut() {
        q[3] = 1.0;
    }
    let mut doc = Document::new(format!("Untitled_HDR{}", s.documents().len() + 1), Size::new(w as u32, h as u32), ColorMode::Rgb, depth);
    doc.icc_profile = if depth == SampleType::F32 { Some(photocraft_cms::builtin::Builtin::LinearSrgb.profile().to_bytes()) } else { icc };
    let fmt = doc.pixel_format();
    let n = fmt.channels();
    let mut data = vec![0.0f32; w * h * n];
    for (q, o) in px.iter().zip(data.chunks_exact_mut(n)) {
        photocraft_raster::from_rgba_into(&fmt, *q, o);
    }
    let mut surf = Surface::new(fmt);
    surf.write_region(area, &data);
    // The merged result is opaque: a real Background, as Crop and Straighten makes.
    let mut bg = Layer::new("Background", LayerContent::Raster(surf));
    bg.locks.transparency = true;
    bg.locks.position = true;
    doc.layers.push(bg);
    let idx = s.add_document(doc, None);
    let ev0 = exposures[order[0]];
    Ok(json!({
        "document": idx,
        "mode": mode,
        "exposureSource": source,
        "ev": exposures.iter().map(|e| (e / ev0).log2()).collect::<Vec<_>>(),
        "offsets": offsets,
        "ghostBase": merged.ghost_base,
        "ghostFraction": merged.ghost_fraction,
        "stops": merged.stops,
        "ms": t0.ms(),
    }))
}

// ---------- Crop and Straighten Photos ----------

/// Paints the pixels outside the selection (holes, gaps between regions) in
/// the median colour of the selection's edge, so detection sees them as
/// background and finds no photo there. Returns false when no pixel is at
/// least half selected (a faint feathered selection), so the caller refuses
/// instead of searching the unmasked area.
fn mask_unselected(sel: &Surface, area: Rect, w: usize, h: usize, rgba: &mut [[f32; 4]]) -> bool {
    let mut cov = Vec::new();
    sel.read_region_into(area, &mut cov);
    let ch = sel.channels().max(1);
    let inside: Vec<bool> = cov.chunks_exact(ch).map(|p| p[0] >= 0.5).collect();
    if inside.len() != w * h || rgba.len() != w * h {
        return true;
    }
    if !inside.iter().any(|&b| b) {
        return false;
    }
    if inside.iter().all(|&b| b) {
        return true;
    }
    let is_in = |x: usize, y: usize| inside.get(y * w + x).copied().unwrap_or(false);
    let mut edge = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let on_edge = x == 0 || y == 0 || x + 1 == w || y + 1 == h || !is_in(x - 1, y) || !is_in(x + 1, y) || !is_in(x, y - 1) || !is_in(x, y + 1);
            if is_in(x, y)
                && on_edge
                && let Some(q) = rgba.get(y * w + x)
            {
                edge.push(*q);
            }
        }
    }
    let bg: [f32; 4] = std::array::from_fn(|c| {
        let mut v: Vec<f32> = edge.iter().map(|p| p[c]).collect();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or(1.0)
    });
    for (q, &keep) in rgba.iter_mut().zip(&inside) {
        if !keep {
            *q = bg;
        }
    }
    true
}

/// Validate the selected layer, then open cropped scan photos without editing the source document.
fn crop_and_straighten(s: &mut Session, _p: &Value) -> Result<Value> {
    crop_ready(s)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let fmt = doc.pixel_format();
    let flat = flattened(&doc, fmt);
    let canvas = doc.bounds();
    // Like Photoshop, only the selected area is searched when there is a selection.
    let area = doc.selection.as_ref().map_or(canvas, |m| m.content_bounds().intersect(&canvas));
    if area.is_empty() {
        return Err(EngineError::Other("the selection is empty".into()));
    }
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut rgba = vec![[0.0f32; 4]; w * h];
    flat.read_rgba_into(area, &mut rgba);
    // Transparent pixels count as background.
    for q in rgba.iter_mut() {
        if q[3] < 1.0 {
            let a = q[3];
            for v in &mut q[..3] {
                *v = *v * a + (1.0 - a);
            }
        }
    }
    if let Some(sel) = &doc.selection
        && !mask_unselected(sel, area, w, h, &mut rgba)
    {
        return Err(EngineError::Other("no pixels are more than 50% selected".into()));
    }
    let mut found = scancrop::find_photos(w, h, &rgba);
    for f in &mut found {
        f.center = [f.center[0] + f64::from(area.x0), f.center[1] + f64::from(area.y0)];
    }
    if found.is_empty() {
        return Err(EngineError::Other("no photos found on a uniform background".into()));
    }
    let mut out = Vec::new();
    for (k, f) in found.iter().enumerate() {
        let (pw, ph) = (f.width.round().max(1.0) as u32, f.height.round().max(1.0) as u32);
        let corners = f.corners();
        let hq = Homography::rect_to_quad([0.0, 0.0, pw as f64, ph as f64], corners).and_then(|h| h.inverse());
        let Some(hm) = hq else { continue };
        let warped = warp_surface(&flat, canvas, &hm, Interp::Bicubic)?;
        let keep = Rect::new(0, 0, pw as i32, ph as i32);
        let mut px = Surface::new(fmt);
        px.write_region(keep, &warped.read_region(keep));
        // Opaque, like a scanned print.
        let n = fmt.channels();
        let mut data = px.read_region(keep);
        for q in data.chunks_exact_mut(n) {
            q[n - 1] = 1.0;
        }
        px.write_region(keep, &data);
        let mut nd = Document::new(format!("{} copy {}", doc.name, k + 1), Size::new(pw, ph), doc.mode, doc.depth);
        nd.icc_profile = doc.icc_profile.clone();
        nd.resolution_dpi = doc.resolution_dpi;
        nd.duotone = doc.duotone.clone();
        let mut bg = Layer::new("Background", LayerContent::Raster(px));
        bg.locks.transparency = true;
        bg.locks.position = true;
        nd.layers.push(bg);
        let idx = s.add_document(nd, None);
        out.push(json!({"document": idx, "width": pw, "height": ph, "angle": f.angle, "center": f.center}));
    }
    Ok(json!({"photos": out}))
}

// ---------- specs ----------

/// Crop needs a selected, visible, pixel-unlocked layer, including inherited state.
fn crop_ready(s: &Session) -> Result<()> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    if st.doc.mode == ColorMode::Indexed {
        return Err(EngineError::Other("not available for Indexed Color documents".into()));
    }
    let id = st.active_layer.ok_or_else(|| EngineError::Other("no layer selected".into()))?;
    let path = st.doc.path_of(id).ok_or(EngineError::NoLayer(id))?;
    if !st.doc.shown_at(&path) {
        return Err(EngineError::Other("the selected layer is hidden".into()));
    }
    crate::commands::check_pixels_unlocked(&st.doc, id)
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.automate.photomerge",
            label: "Photomerge…",
            menu: &["File", "Automate"],
            shortcut: None,
            params: r##"{"paths":[str]|folder,"useOpenDocuments":bool=false,"layout":"auto|perspective|cylindrical|spherical|collage|reposition","blend":bool=true,"vignetteRemoval":bool=false,"geometricCorrection":bool=false,"contentAwareFill":bool=false,"focalLength":mm=0 (35 mm equivalent; 0 = EXIF or estimated),"memoryBudget":bytes?=half of RAM} → new document, one masked layer per image"##,
            enabled: always,
            run: photomerge,
            journal: true,
        },
        CommandSpec {
            id: "file.automate.mergeToHdrPro",
            label: "Merge to HDR Pro…",
            menu: &["File", "Automate"],
            shortcut: None,
            params: r##"{"paths":[str]|folder,"useOpenDocuments":bool=false,"exposures":[ev]? (default EXIF, else estimated),"align":bool=true,"removeGhosts":bool=false,"ghostBase":int?,"mode":"32|16|8","method":"localAdaptation|exposureGamma|highlightCompression|equalizeHistogram","radius":1..500=7,"strength":0.1..4=0.52,"gamma":0.1..2=1,"exposure":-5..5=0,"detail":-100..300=30,"shadow":-100..100=0,"highlight":-100..100=0,"vibrance":-100..100=0,"saturation":-100..100=20} → new document (32-bit linear, or tone-mapped 16/8-bit)"##,
            enabled: always,
            run: merge_to_hdr,
            journal: true,
        },
        CommandSpec {
            id: "file.automate.cropAndStraightenPhotos",
            label: "Crop and Straighten Photos",
            menu: &["File", "Automate"],
            shortcut: None,
            params: "{} → one new document per photo found on the scan (inside the selection when there is one)",
            enabled: |s| crop_ready(s).map_err(|e| e.to_string()),
            run: crop_and_straighten,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;
