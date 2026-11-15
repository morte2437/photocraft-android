//! Home screen Recent Files Thumbnail Card Grid and List views.
//!
//! Provides thumbnail preview loading and caching, interactive file cards with
//! thumbnail image, document icon, filename, and file path/location, along with
//! Grid/List view switching matching Photoshop 2026 Home screen.

use std::collections::HashMap;
use std::path::Path;

use egui::{Color32, ColorImage, Rect, Response, Sense, Stroke, TextureHandle, TextureOptions, Ui, Vec2, pos2, vec2};

use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// Cache key identifying the origin state of a loaded thumbnail.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CacheKey {
    /// Rendered from an active in-memory document with a specific revision.
    OpenDoc { revision: u64 },
    /// Read from a file on disk with modification time and file size.
    #[cfg(not(target_arch = "wasm32"))]
    DiskFile { modified: Option<std::time::SystemTime>, len: u64 },
}

#[derive(Clone)]
pub(crate) struct ThumbnailEntry {
    pub(crate) key: CacheKey,
    pub(crate) texture: Option<TextureHandle>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) last_checked: std::time::Instant,
}

/// Cache for loaded recent file preview textures.
#[derive(Default)]
pub(crate) struct ThumbnailCache {
    pub(crate) entries: HashMap<String, ThumbnailEntry>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) was_focused: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) in_flight: HashMap<String, u64>,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) next_req_id: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) pending: Vec<(String, u64, CacheKey, Option<ColorImage>)>,
}

#[cfg(not(target_arch = "wasm32"))]
const MAX_CONCURRENT_THUMBNAIL_LOADS: usize = 1;

/// Minimum time between two live-document thumbnail compositions for the same path.
#[cfg(all(not(target_arch = "wasm32"), not(test)))]
const OPEN_DOC_REGEN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);
#[cfg(all(not(target_arch = "wasm32"), test))]
const OPEN_DOC_REGEN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

#[cfg(not(target_arch = "wasm32"))]
fn load_thumbnail_catching_panic(load: impl FnOnce() -> Option<ColorImage>) -> Option<ColorImage> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(load)).ok().flatten()
}

impl ThumbnailCache {
    /// Invalidate the cached thumbnail for a single path (e.g. after save, open, or remove).
    pub(crate) fn invalidate(&mut self, path: &str) {
        self.entries.remove(path);
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.in_flight.remove(path);
            self.pending.retain(|(p, ..)| p != path);
        }
    }

    /// Clear all cached thumbnails (e.g. on Clear Recent Files).
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.in_flight.clear();
            self.pending.clear();
        }
    }
}

/// Check if two path strings match, tolerating slash direction and Windows case differences.
fn paths_match(a: &str, b: &str) -> bool {
    #[cfg(windows)]
    {
        a == b || a.replace('\\', "/").eq_ignore_ascii_case(&b.replace('\\', "/"))
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

/// Downscale an RGBA8 buffer to fit within `max_dim` on its longest side.
#[cfg(not(target_arch = "wasm32"))]
fn downsample_rgba(w: u32, h: u32, src_data: &[u8], max_dim: u32) -> Option<ColorImage> {
    if w == 0 || h == 0 || src_data.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    let (tw, th) = if w > max_dim || h > max_dim {
        let scale = (max_dim as f32) / (w.max(h) as f32);
        (((w as f32) * scale).round().max(1.0) as u32, ((h as f32) * scale).round().max(1.0) as u32)
    } else {
        (w, h)
    };
    if tw == w && th == h {
        Some(ColorImage::from_rgba_unmultiplied([w as usize, h as usize], src_data))
    } else {
        let mut thumb_px = vec![0u8; (tw as usize) * (th as usize) * 4];
        for y in 0..th {
            let sy = ((y as f32 / th as f32) * h as f32) as usize;
            for x in 0..tw {
                let sx = ((x as f32 / tw as f32) * w as f32) as usize;
                let src_idx = (sy * w as usize + sx) * 4;
                let dst_idx = (y as usize * tw as usize + x as usize) * 4;
                if src_idx + 4 <= src_data.len()
                    && dst_idx + 4 <= thumb_px.len()
                    && let Some(src) = src_data.get(src_idx..src_idx + 4)
                    && let Some(dst) = thumb_px.get_mut(dst_idx..dst_idx + 4)
                {
                    dst.copy_from_slice(src);
                }
            }
        }
        Some(ColorImage::from_rgba_unmultiplied([tw as usize, th as usize], &thumb_px))
    }
}

/// Read a PSD/PSB thumbnail preview by streaming only the header and image resources section.
///
/// Bounded range reader: inspects the 26-byte header and reads only the resource blocks
/// section (up to 32 MB), extracting resource 1036 (JFIF JPEG) or 1033 without reading
/// the layer records, channel data, or multi-gigabyte merged payload. If no embedded thumbnail
/// exists, fallback composite decoding is only permitted if header dimensions are <= 2048x2048
/// and the total file size on disk is <= 64 MB.
#[cfg(not(target_arch = "wasm32"))]
fn read_psd_thumbnail_bounded(path: &Path, file_len: u64) -> Option<ColorImage> {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    let mut file = File::open(path).ok()?;

    // 1. Read 26-byte header
    let mut header = [0u8; 26];
    file.read_exact(&mut header).ok()?;

    if &header[0..4] != b"8BPS" {
        return None;
    }
    let version = u16::from_be_bytes([header[4], header[5]]);
    if version != 1 && version != 2 {
        return None;
    }
    let channels = u16::from_be_bytes([header[12], header[13]]);
    let height = u32::from_be_bytes([header[14], header[15], header[16], header[17]]);
    let width = u32::from_be_bytes([header[18], header[19], header[20], header[21]]);
    let depth = u16::from_be_bytes([header[22], header[23]]);

    // 2. Read color mode data length (4 bytes)
    let mut cmd_len_buf = [0u8; 4];
    file.read_exact(&mut cmd_len_buf).ok()?;
    let cmd_len = u32::from_be_bytes(cmd_len_buf);
    if cmd_len > 16 * 1024 * 1024 {
        return None;
    }
    file.seek(SeekFrom::Current(cmd_len as i64)).ok()?;

    // 3. Read image resources section length (4 bytes)
    let mut res_len_buf = [0u8; 4];
    file.read_exact(&mut res_len_buf).ok()?;
    let res_len = u32::from_be_bytes(res_len_buf);

    // 4. Read only the resources section (bounded up to 32 MB and within file length)
    if res_len > 0 && res_len <= 32 * 1024 * 1024 && (res_len as u64) <= file_len {
        let mut res_bytes = vec![0u8; res_len as usize];
        if file.read_exact(&mut res_bytes).is_ok()
            && let Ok((resources, _)) = craft_psd::tiff::resources_from_bytes(&res_bytes)
        {
            let decode_opts = photocraft_codecs::DecodeOptions {
                limits: photocraft_codecs::Limits { max_width: 4096, max_height: 4096, max_pixels: 4096 * 4096, max_alloc: 4096 * 4096 * 4 },
                ..Default::default()
            };
            for r in &resources {
                if (r.id == craft_psd::resources::ids::THUMBNAIL || r.id == craft_psd::resources::ids::THUMBNAIL_PS4)
                    && let Some(jfif) = r.data.get(28..)
                    && let Ok(img) = photocraft_codecs::decode_with(jfif, &decode_opts)
                {
                    let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
                    if rgba.data().len() == rgba.width() as usize * rgba.height() as usize * 4 {
                        return downsample_rgba(rgba.width(), rgba.height(), rgba.data(), 256);
                    }
                }
            }
        }
    }

    // 5. Fallback for PSDs without embedded JFIF preview:
    // First, check header dimensions: must be <= 2048x2048 before reading any more of the file!
    if width == 0 || height == 0 || width > 2048 || height > 2048 {
        return None;
    }

    // Second, enforce input budget: if the file size on disk exceeds 64 MB, do NOT allocate/read the whole document
    const MAX_FALLBACK_FILE_SIZE: u64 = 64 * 1024 * 1024;
    if file_len > MAX_FALLBACK_FILE_SIZE {
        return None;
    }

    // Third, guard against uncompressed memory spikes / bombs from high channel counts or 32-bit depth:
    let sample_bytes: u64 = match depth {
        1 | 8 => 1,
        16 => 2,
        32 => 4,
        _ => return None,
    };
    const MAX_FALLBACK_DECODED_BYTES: u64 = 64 * 1024 * 1024;
    let decoded_bytes = (width as u64).checked_mul(height as u64)?.checked_mul(channels as u64)?.checked_mul(sample_bytes)?;
    if decoded_bytes > MAX_FALLBACK_DECODED_BYTES {
        return None;
    }

    // Fourth, within budget and safe dimensions: read file and decode merged composite
    let bytes = std::fs::read(path).ok()?;
    if let Ok(file) = craft_psd::PsdFile::from_bytes(&bytes)
        && let Ok(comp) = photocraft_io::merged_composite(&file)
    {
        let w = file.header.width;
        let h = file.header.height;
        if w > 0 && h > 0 && comp.len() == (w as usize) * (h as usize) {
            let mut raw_bytes = Vec::with_capacity(comp.len() * 4);
            for &[r, g, b, a] in comp.iter() {
                raw_bytes.push((r.clamp(0.0, 1.0) * 255.0).round() as u8);
                raw_bytes.push((g.clamp(0.0, 1.0) * 255.0).round() as u8);
                raw_bytes.push((b.clamp(0.0, 1.0) * 255.0).round() as u8);
                raw_bytes.push((a.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            return downsample_rgba(w, h, &raw_bytes, 256);
        }
    }

    None
}

/// Load thumbnail image with bounded streaming I/O and strict size limits.
#[cfg(not(target_arch = "wasm32"))]
fn load_thumbnail_image_bounded(path: &str, file_len: u64) -> Option<ColorImage> {
    let std_path = Path::new(path);
    let ext = std_path.extension().and_then(|e| e.to_str()).unwrap_or("");

    // 1. PSD / PSB: read header and resources with bounded range reading
    if ext.eq_ignore_ascii_case("psd") || ext.eq_ignore_ascii_case("psb") {
        return read_psd_thumbnail_bounded(std_path, file_len);
    }

    // Enforce 64 MB input budget for all other file types (.pcraft, PNG, JPEG, TIFF, etc.)
    const MAX_THUMBNAIL_FILE_SIZE: u64 = 64 * 1024 * 1024;
    if file_len > MAX_THUMBNAIL_FILE_SIZE {
        return None;
    }

    // Guard against decompression bombs: bound decoder allocation and dimensions up front
    let decode_opts = photocraft_codecs::DecodeOptions {
        limits: photocraft_codecs::Limits { max_width: 4096, max_height: 4096, max_pixels: 4096 * 4096, max_alloc: 4096 * 4096 * 4 },
        ..Default::default()
    };

    // 2. If .pcraft, read the embedded thumbnail PNG
    if ext.eq_ignore_ascii_case("pcraft") {
        if std_path.is_dir() {
            let thumb_path = std_path.join("thumb.png");
            if let Ok(thumb_meta) = std::fs::metadata(&thumb_path)
                && thumb_meta.len() <= MAX_THUMBNAIL_FILE_SIZE
                && let Ok(bytes) = std::fs::read(&thumb_path)
                && let Ok(img) = photocraft_codecs::decode_with(&bytes, &decode_opts)
            {
                let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
                if rgba.data().len() == rgba.width() as usize * rgba.height() as usize * 4 {
                    return downsample_rgba(rgba.width(), rgba.height(), rgba.data(), 256);
                }
            }
        } else if let Ok(bytes) = std::fs::read(std_path)
            && let Ok(Some(png_bytes)) = photocraft_format::read_thumbnail(&bytes)
            && let Ok(img) = photocraft_codecs::decode_with(&png_bytes, &decode_opts)
        {
            let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            if rgba.data().len() == rgba.width() as usize * rgba.height() as usize * 4 {
                return downsample_rgba(rgba.width(), rgba.height(), rgba.data(), 256);
            }
        }
        return None;
    }

    // 3. Flat raster formats (PNG, JPG, WebP, TIFF, GIF, BMP, etc.)
    if let Ok(bytes) = std::fs::read(std_path)
        && let Ok(img) = photocraft_codecs::decode_with(&bytes, &decode_opts)
    {
        let (w, h) = img.dimensions();
        // Guard against decompression bombs: don't expand > 4096 images to gigabytes of RGBA
        if w > 0 && h > 0 && w <= 4096 && h <= 4096 {
            let rgba = img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8);
            return downsample_rgba(w, h, rgba.data(), 256);
        }
    }

    None
}

/// Drain completed asynchronous thumbnail results and update the texture cache on the UI thread.
#[cfg(not(target_arch = "wasm32"))]
fn drain_pending_thumbnails(app: &PhotocraftApp, ctx: &egui::Context, opts: TextureOptions) {
    let pending: Vec<(String, u64, CacheKey, Option<ColorImage>)> = {
        let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if cache.pending.is_empty() {
            return;
        }
        std::mem::take(&mut cache.pending)
    };

    let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for (path, req_id, key, opt_img) in pending {
        if cache.in_flight.get(&path).copied() == Some(req_id) {
            cache.in_flight.remove(&path);
        } else {
            continue;
        }
        // If the document is currently open in the session, do not overwrite its live in-memory thumbnail
        if app.session.documents().iter().any(|d| d.path.as_deref().is_some_and(|p| paths_match(p, &path))) {
            continue;
        }
        let tex = opt_img.map(|ci| {
            if let Some(existing) = cache.entries.get_mut(&path).and_then(|e| e.texture.as_mut()) {
                existing.set(ci, opts);
                existing.clone()
            } else {
                ctx.load_texture(format!("recent_thumb_{path}"), ci, opts)
            }
        });
        cache.entries.insert(path, ThumbnailEntry { key, texture: tex, last_checked: std::time::Instant::now() });
    }
}

/// Retrieve, lazily load, or regenerate the preview texture for `path`.
///
/// If `path` is currently open in `app.session`, the thumbnail is rendered from
/// the live document composite and re-rendered whenever its revision changes.
/// If `path` is closed, the thumbnail is decoded from disk in the background (or inline
/// in non-background test environments), with bounded range reads and input budgets.
pub(crate) fn get_or_load_thumbnail(app: &PhotocraftApp, ctx: &egui::Context, path: &str) -> Option<TextureHandle> {
    let opts = TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: Some(egui::TextureFilter::Linear),
    };

    #[cfg(not(target_arch = "wasm32"))]
    drain_pending_thumbnails(app, ctx, opts);

    // 1. Is this document currently open in the session?
    if let Some(st) = app.session.documents().iter().find(|d| d.path.as_deref().is_some_and(|p| paths_match(p, path))) {
        let current_key = CacheKey::OpenDoc { revision: st.revision };
        let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(entry) = cache.entries.get(path)
            && entry.key == current_key
        {
            return entry.texture.clone();
        }

        // Composing the live document runs on the UI thread, so while the document is
        // being edited regenerate at most once per interval and show the previous preview
        // in between. A repaint is scheduled so the final revision is still picked up.
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(entry) = cache.entries.get(path)
            && matches!(entry.key, CacheKey::OpenDoc { .. })
            && entry.texture.is_some()
        {
            let since = entry.last_checked.elapsed();
            if since < OPEN_DOC_REGEN_INTERVAL {
                ctx.request_repaint_after(OPEN_DOC_REGEN_INTERVAL.saturating_sub(since));
                return entry.texture.clone();
            }
        }

        let rgba = photocraft_compose::thumbnail(&st.doc, 256);
        let color_image = ColorImage::from_rgba_unmultiplied([rgba.width as usize, rgba.height as usize], &rgba.pixels);

        let tex = if let Some(existing) = cache.entries.get_mut(path).and_then(|e| e.texture.as_mut()) {
            existing.set(color_image, opts);
            existing.clone()
        } else {
            ctx.load_texture(format!("recent_thumb_{path}"), color_image, opts)
        };

        cache.entries.insert(
            path.to_string(),
            ThumbnailEntry {
                key: current_key,
                texture: Some(tex.clone()),
                #[cfg(not(target_arch = "wasm32"))]
                last_checked: std::time::Instant::now(),
            },
        );
        return Some(tex);
    }

    // 2. Otherwise inspect the file on disk
    #[cfg(not(target_arch = "wasm32"))]
    {
        #[cfg(test)]
        const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
        #[cfg(not(test))]
        const RECHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3);

        let (focus_gained, cached_entry, is_in_flight) = {
            let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let focused = ctx.input(|i| i.focused);
            let focus_gained = focused && !cache.was_focused;
            if focus_gained {
                cache.was_focused = true;
            } else if !focused {
                cache.was_focused = false;
            }

            let entry = cache.entries.get(path).cloned();
            let in_flight = cache.in_flight.contains_key(path);
            (focus_gained, entry, in_flight)
        };

        // Fast path: if already cached and neither focus gained nor recheck interval elapsed,
        // return the texture immediately from memory without touching the filesystem.
        if !focus_gained
            && let Some(entry) = &cached_entry
            && entry.last_checked.elapsed() < RECHECK_INTERVAL
        {
            return entry.texture.clone();
        }

        let std_path = Path::new(path);
        let Ok(meta) = std::fs::metadata(std_path) else {
            let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.entries.remove(path);
            cache.in_flight.remove(path);
            return None;
        };

        let current_key = CacheKey::DiskFile { modified: meta.modified().ok(), len: meta.len() };

        if let Some(entry) = &cached_entry
            && entry.key == current_key
        {
            let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(e) = cache.entries.get_mut(path) {
                e.last_checked = std::time::Instant::now();
            }
            return entry.texture.clone();
        }

        if is_in_flight {
            return cached_entry.and_then(|e| e.texture);
        }

        if app.background_jobs {
            let req_id = {
                let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if cache.in_flight.len() >= MAX_CONCURRENT_THUMBNAIL_LOADS {
                    return cached_entry.and_then(|e| e.texture);
                }
                let id = cache.next_req_id;
                cache.next_req_id = cache.next_req_id.wrapping_add(1);
                cache.in_flight.insert(path.to_string(), id);
                id
            };
            let cache_arc = std::sync::Arc::clone(&app.thumbnail_cache);
            let ctx_clone = ctx.clone();
            let path_str = path.to_string();
            let key_clone = current_key;
            let file_len = meta.len();
            std::thread::spawn(move || {
                let opt_img = load_thumbnail_catching_panic(|| load_thumbnail_image_bounded(&path_str, file_len));
                let mut cache = cache_arc.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                // Only enqueue if this request generation is still the active one
                if cache.in_flight.get(&path_str).copied() == Some(req_id) {
                    cache.pending.push((path_str, req_id, key_clone, opt_img));
                    ctx_clone.request_repaint();
                }
            });
            cached_entry.and_then(|e| e.texture)
        } else {
            let opt_img = load_thumbnail_image_bounded(path, meta.len());
            let tex = opt_img.map(|ci| {
                let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(existing) = cache.entries.get_mut(path).and_then(|e| e.texture.as_mut()) {
                    existing.set(ci, opts);
                    existing.clone()
                } else {
                    ctx.load_texture(format!("recent_thumb_{path}"), ci, opts)
                }
            });
            let mut cache = app.thumbnail_cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.entries.insert(path.to_string(), ThumbnailEntry { key: current_key, texture: tex.clone(), last_checked: std::time::Instant::now() });
            tex
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = ctx;
        let _ = path;
        None
    }
}

/// Action output from the recent files view.
pub(crate) enum RecentAction {
    Open(String),
    Remove(String),
    Clear,
    OpenDialog,
}

/// Card width in the grid.
pub const GRID_CARD_WIDTH: f32 = 184.0;
/// Card height in the grid.
pub const GRID_CARD_HEIGHT: f32 = 244.0;
/// Preview area height within a grid card.
pub const GRID_PREVIEW_HEIGHT: f32 = 168.0;
/// Row height in list view.
pub const LIST_ROW_HEIGHT: f32 = 34.0;

/// Render the header for Recent files with title and Grid/List toggle buttons.
pub(crate) fn recent_header(ui: &mut Ui, is_grid: &mut bool, total_width: f32) {
    let t = Tokens::get(ui.ctx());
    let (header_rect, _) = ui.allocate_exact_size(vec2(total_width, 28.0), Sense::hover());

    // Left label: "Recent"
    let label_galley = ui.painter().layout_no_wrap(tl!("Recent").to_string(), crate::theme::semibold(17.0), t.text);
    let label_pos = pos2(header_rect.left(), header_rect.center().y - label_galley.size().y / 2.0);
    ui.painter().galley(label_pos, label_galley, t.text);
    let label_rect = Rect::from_min_size(label_pos, vec2(100.0, 24.0));
    let label_resp = ui.interact(label_rect, ui.scope_id().with("recent_title"), Sense::hover());
    label_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, tl!("Recent")));

    // Container box for the toggles, aligned flush to the right edge of header_rect
    let box_size = vec2(60.0, 28.0);
    let box_rect = Rect::from_min_size(pos2(header_rect.right() - box_size.x, header_rect.top()), box_size);
    ui.painter().rect_filled(box_rect, 4.0, t.field);
    ui.painter().rect_stroke(box_rect, 4.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);

    let grid_rect = Rect::from_min_size(box_rect.min + vec2(2.0, 2.0), vec2(28.0, 24.0));
    let list_rect = Rect::from_min_size(pos2(box_rect.min.x + 30.0, box_rect.min.y + 2.0), vec2(28.0, 24.0));

    let grid_resp = ui.interact(grid_rect, ui.scope_id().with("toggle_grid"), Sense::click());
    let list_resp = ui.interact(list_rect, ui.scope_id().with("toggle_list"), Sense::click());

    grid_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Grid view")));
    list_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("List view")));

    let grid_resp = grid_resp.on_hover_text(tl!("Grid view")).on_hover_cursor(egui::CursorIcon::PointingHand);
    let list_resp = list_resp.on_hover_text(tl!("List view")).on_hover_cursor(egui::CursorIcon::PointingHand);

    if *is_grid {
        ui.painter().rect_filled(grid_rect, 3.0, t.hover);
        ui.painter().rect_stroke(grid_rect, 3.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }
    if !*is_grid {
        ui.painter().rect_filled(list_rect, 3.0, t.hover);
        ui.painter().rect_stroke(list_rect, 3.0, Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
    }

    let grid_tint = if *is_grid { t.text } else { t.text_dim };
    let list_tint = if !*is_grid { t.text } else { t.text_dim };

    icons::paint(ui, grid_rect, "grid-2x2", 15.0, grid_tint);
    icons::paint(ui, list_rect, "marquee-row", 15.0, list_tint);

    if grid_resp.clicked() {
        *is_grid = true;
    }
    if list_resp.clicked() {
        *is_grid = false;
    }
}

/// Render a single visual thumbnail card in grid view.
pub(crate) fn render_grid_card(ui: &mut Ui, path: &str, name: &str, folder: &str, thumb: Option<&TextureHandle>) -> (Response, Option<RecentAction>) {
    let t = Tokens::get(ui.ctx());
    let (card_rect, resp) = ui.allocate_exact_size(vec2(GRID_CARD_WIDTH, GRID_CARD_HEIGHT), Sense::click());
    let hovered = ui.rect_contains_pointer(card_rect);

    // Card background & border
    let bg_fill = t.card;
    ui.painter().rect_filled(card_rect, 6.0, bg_fill);
    let border_stroke = if hovered { Stroke::new(1.0, t.accent) } else { Stroke::new(1.0, t.card_border) };
    ui.painter().rect_stroke(card_rect, 6.0, border_stroke, egui::StrokeKind::Inside);

    // Preview area
    let preview_rect = Rect::from_min_size(card_rect.min + vec2(1.0, 1.0), vec2(GRID_CARD_WIDTH - 2.0, GRID_PREVIEW_HEIGHT));
    // Inner well for preview area with rounded top corners
    ui.painter().rect_filled(preview_rect, egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 }, t.field);

    if let Some(tex) = thumb {
        let tex_size = tex.size_vec2();
        let avail_size = preview_rect.size() - vec2(8.0, 8.0);
        if tex_size.x > 0.0 && tex_size.y > 0.0 {
            let aspect = tex_size.x / tex_size.y;
            let target_w = (avail_size.y * aspect).min(avail_size.x);
            let target_h = target_w / aspect;
            let img_rect = Rect::from_center_size(preview_rect.center(), vec2(target_w, target_h));
            ui.painter().image(tex.id(), img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
    } else {
        // Fallback document vector icon
        let icon_box = Rect::from_center_size(preview_rect.center(), Vec2::splat(40.0));
        icons::paint(ui, icon_box, "file", 34.0, t.text_faint);
    }

    // Small photo icon badge at bottom-left of preview
    let badge_rect = Rect::from_center_size(pos2(preview_rect.left() + 16.0, preview_rect.bottom() - 14.0), vec2(16.0, 16.0));
    icons::paint(ui, badge_rect, "image", 13.0, t.text_dim);

    // Bottom info area
    let info_top = preview_rect.bottom() + 8.0;
    let text_x = card_rect.left() + 14.0;
    let text_max_w = (card_rect.right() - text_x - 12.0).max(10.0);

    // Filename
    let name_galley = crate::tab_strip::elided(ui, name, crate::theme::semibold(13.0), t.text, text_max_w);
    ui.painter().galley(pos2(text_x, info_top), name_galley, t.text);

    // Monitor icon + folder / "On your computer"
    let monitor_rect = Rect::from_center_size(pos2(text_x + 6.0, info_top + 28.0), vec2(14.0, 14.0));
    icons::paint(ui, monitor_rect, "monitor", 12.0, t.text_faint);

    let folder_display = if folder.is_empty() { tl!("On your computer").to_owned() } else { folder.to_string() };
    let folder_galley = crate::tab_strip::elided(ui, &folder_display, egui::FontId::proportional(11.0), t.text_faint, text_max_w - 18.0);
    ui.painter().galley(pos2(text_x + 18.0, info_top + 22.0), folder_galley, t.text_faint);

    // Hover 'x' close button in top right of card
    let mut action = None;
    if hovered {
        let close_rect = Rect::from_center_size(pos2(card_rect.right() - 14.0, card_rect.top() + 14.0), vec2(20.0, 20.0));
        let x_resp = ui.interact(close_rect, resp.id.with("remove"), Sense::click());
        let label = crate::i18n::fmt(tl!("Remove {name} from Recent"), &[("name", name)]);
        x_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        ui.painter().rect_filled(close_rect, 10.0, Color32::from_black_alpha(190));
        let x_color = if x_resp.hovered() { Color32::WHITE } else { t.text_dim };
        icons::paint(ui, close_rect, "x", 11.0, x_color);
        if x_resp.on_hover_text(tl!("Remove from Recent")).clicked() {
            action = Some(RecentAction::Remove(path.to_string()));
        }
    }

    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    let resp = resp.on_hover_text(path).on_hover_cursor(egui::CursorIcon::PointingHand);

    if resp.clicked() && action.is_none() {
        action = Some(RecentAction::Open(path.to_string()));
    }

    (resp, action)
}

/// Render a dashed/hairline drop target card inside the recent grid.
pub(crate) fn render_inline_drop_card(ui: &mut Ui, drop_text: &str) -> bool {
    let t = Tokens::get(ui.ctx());
    let (card_rect, resp) = ui.allocate_exact_size(vec2(GRID_CARD_WIDTH, GRID_CARD_HEIGHT), Sense::click());
    let hovered = ui.rect_contains_pointer(card_rect);

    let border_color = if hovered { t.accent } else { t.separator };
    let bg_color = if hovered { t.hover } else { Color32::TRANSPARENT };

    ui.painter().rect_filled(card_rect, 6.0, bg_color);
    ui.painter().rect_stroke(card_rect, 6.0, Stroke::new(1.0, border_color), egui::StrokeKind::Inside);

    // Centered plus / image icon
    let icon_box = Rect::from_center_size(pos2(card_rect.center().x, card_rect.center().y - 20.0), vec2(36.0, 36.0));
    let icon_color = if hovered { t.text } else { t.text_dim };
    icons::paint(ui, icon_box, "image", 28.0, icon_color);

    // Message
    let max_w = card_rect.width() - 24.0;
    let text_color = if hovered { t.text } else { t.text_faint };
    let job = egui::text::LayoutJob::simple(drop_text.to_string(), egui::FontId::proportional(11.5), text_color, max_w);
    let galley = ui.painter().layout_job(job);
    let text_pos = pos2(card_rect.center().x - galley.size().x / 2.0, card_rect.center().y + 8.0);
    ui.painter().galley(text_pos, galley, text_color);

    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Open…")));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tl!("Open…")).clicked()
}

fn fit_image_rect(image_size: Vec2, bounds: Rect) -> Rect {
    if image_size.x > 0.0 && image_size.y > 0.0 {
        let scale = (bounds.width() / image_size.x).min(bounds.height() / image_size.y);
        Rect::from_center_size(bounds.center(), image_size * scale)
    } else {
        bounds
    }
}

/// Render a single row in list view with thumbnail icon.
pub(crate) fn render_list_row(
    ui: &mut Ui,
    path: &str,
    name: &str,
    folder: &str,
    thumb: Option<&TextureHandle>,
    row_width: f32,
) -> (Response, Option<RecentAction>) {
    let t = Tokens::get(ui.ctx());
    let (row, resp) = ui.allocate_exact_size(vec2(row_width, LIST_ROW_HEIGHT), Sense::click());
    let hovered = ui.rect_contains_pointer(row);
    if hovered {
        ui.painter().rect_filled(row, t.radius_sm, t.hover);
    }

    let close_rect = Rect::from_center_size(pos2(row.right() - 16.0, row.center().y), vec2(22.0, 22.0));
    let right_limit = if hovered { close_rect.left() - 4.0 } else { row.right() - 10.0 };

    // Thumbnail icon on the left (24x24)
    let thumb_box = Rect::from_center_size(pos2(row.left() + 18.0, row.center().y), vec2(24.0, 24.0));
    ui.painter().rect_filled(thumb_box, t.radius_sm, t.field);
    if let Some(tex) = thumb {
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        ui.painter().image(tex.id(), fit_image_rect(tex.size_vec2(), thumb_box), uv, Color32::WHITE);
    } else {
        icons::paint(ui, thumb_box, "file", 14.0, t.text_dim);
    }

    let text_x = row.left() + 38.0;
    let name_g = crate::tab_strip::elided(ui, name, crate::theme::semibold(12.5), t.text, 220.0);
    let name_w = name_g.size().x;
    ui.painter().galley(pos2(text_x, row.center().y - name_g.size().y / 2.0), name_g, t.text);

    let folder_max = (right_limit - (text_x + name_w + 14.0)).max(0.0);
    if folder_max > 20.0 && !folder.is_empty() {
        let fg = crate::tab_strip::elided(ui, folder, egui::FontId::proportional(11.5), t.text_faint, folder_max);
        ui.painter().galley(pos2(right_limit - fg.size().x, row.center().y - fg.size().y / 2.0), fg, t.text_faint);
    }

    let mut action = None;
    if hovered {
        let x_resp = ui.interact(close_rect, resp.id.with("remove"), Sense::click());
        let label = crate::i18n::fmt(tl!("Remove {name} from Recent"), &[("name", name)]);
        x_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
        let color = icons::button_chrome(ui, close_rect, false, x_resp.hovered());
        icons::paint(ui, close_rect, "x", 12.0, color);
        if x_resp.on_hover_text(tl!("Remove from Recent")).clicked() {
            action = Some(RecentAction::Remove(path.to_string()));
        }
    }

    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    let resp = resp.on_hover_text(path).on_hover_cursor(egui::CursorIcon::PointingHand);

    if resp.clicked() && action.is_none() {
        action = Some(RecentAction::Open(path.to_string()));
    }

    (resp, action)
}

/// Judge a click that opens the recent file `path` as File › Open Recent's item for it, as the
/// menu does: synthetic input from a client confined to its automation roots can't plant a recent
/// file outside them and click its Home card open. A user's own click is always allowed.
fn open_recent_gate(app: &PhotocraftApp, path: &str) -> Result<(), String> {
    let Some(authorize) = app.services.automation_command.as_ref() else { return Ok(()) };
    if !app.automation_input {
        return Ok(());
    }
    // The Home list is File › Open Recent's first entries, so the card's index there is its id.
    let i = app.ui.recent_files.iter().position(|p| p == path).ok_or_else(|| "no such recent file".to_string())?;
    authorize(&format!("file.openRecent.{i}"), &serde_json::json!({}))
}

/// The main entry point to render the Home Recent Files visual grid or list.
pub(crate) fn show_home_recent(app: &mut PhotocraftApp, ui: &mut Ui, recent: &[String], avail_width: f32) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let opts = TextureOptions {
            magnification: egui::TextureFilter::Linear,
            minification: egui::TextureFilter::Linear,
            wrap_mode: egui::TextureWrapMode::ClampToEdge,
            mipmap_mode: Some(egui::TextureFilter::Linear),
        };
        drain_pending_thumbnails(app, ui.ctx(), opts);
    }

    let is_grid = app.ui.chrome.home_recent_grid;
    let content_width = avail_width;

    let mut toggle_grid = is_grid;
    recent_header(ui, &mut toggle_grid, content_width);
    if toggle_grid != is_grid {
        app.ui.chrome.home_recent_grid = toggle_grid;
    }

    ui.add_space(14.0);

    if recent.is_empty() {
        let t = Tokens::get(ui.ctx());
        ui.label(egui::RichText::new(tl!("No recent files")).color(t.text_dim).size(13.0));
        return;
    }

    let mut action_to_take = None;

    if is_grid {
        let spacing = 16.0;
        let card_w = GRID_CARD_WIDTH;
        let cols = ((content_width + spacing) / (card_w + spacing)).floor().max(1.0) as usize;

        egui::Grid::new("home_recent_grid_layout").spacing(vec2(spacing, spacing)).show(ui, |ui| {
            for (idx, path) in recent.iter().enumerate() {
                let name = crate::file_open::display_name(path);
                let folder = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                let thumb = get_or_load_thumbnail(app, ui.ctx(), path);

                let (resp, action) = render_grid_card(ui, path, &name, &folder, thumb.as_ref());
                if let Some(act) = action {
                    action_to_take = Some(act);
                }

                let path_clone = path.clone();
                resp.context_menu(|ui| {
                    if ui.button(tl!("Remove from Recent")).clicked() {
                        action_to_take = Some(RecentAction::Remove(path_clone));
                        ui.close();
                    }
                    if ui.button(tl!("Clear Recent Files")).clicked() {
                        action_to_take = Some(RecentAction::Clear);
                        ui.close();
                    }
                });

                if (idx + 1) % cols == 0 {
                    ui.end_row();
                }
            }

            // Inline drop card as the last card in the grid
            let drop_idx = recent.len();
            let drop_hint = crate::canvas::start_screen_drop_hint(app);
            if render_inline_drop_card(ui, &drop_hint) {
                action_to_take = Some(RecentAction::OpenDialog);
            }
            if (drop_idx + 1).is_multiple_of(cols) {
                ui.end_row();
            }
        });
    } else {
        for path in recent {
            let name = crate::file_open::display_name(path);
            let folder = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            let thumb = get_or_load_thumbnail(app, ui.ctx(), path);

            let (resp, action) = render_list_row(ui, path, &name, &folder, thumb.as_ref(), content_width);
            if let Some(act) = action {
                action_to_take = Some(act);
            }

            let path_clone = path.clone();
            resp.context_menu(|ui| {
                if ui.button(tl!("Remove from Recent")).clicked() {
                    action_to_take = Some(RecentAction::Remove(path_clone));
                    ui.close();
                }
                if ui.button(tl!("Clear Recent Files")).clicked() {
                    action_to_take = Some(RecentAction::Clear);
                    ui.close();
                }
            });
        }
    }

    if let Some(action) = action_to_take {
        match action {
            RecentAction::Open(path) => {
                if let Err(e) = open_recent_gate(app, &path).and_then(|()| app.open_path(&path)) {
                    app.open_failed(&crate::file_open::display_name(&path), &e);
                }
            }
            RecentAction::Remove(path) => {
                app.remove_recent(&path);
            }
            RecentAction::Clear => {
                app.clear_recent();
            }
            RecentAction::OpenDialog => {
                let _ = app.open_dialog_file();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn list_thumbnail_fit_preserves_aspect_ratio() {
        let bounds = Rect::from_min_size(pos2(0.0, 0.0), vec2(24.0, 24.0));
        let fitted = fit_image_rect(vec2(256.0, 144.0), bounds);
        assert_eq!(fitted.size(), vec2(24.0, 13.5));
        assert_eq!(fitted.center(), bounds.center());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn panicking_thumbnail_load_returns_no_preview() {
        let loaded = load_thumbnail_catching_panic(|| panic!("simulated decoder panic"));
        assert!(loaded.is_none());
    }

    #[test]
    fn recent_grid_and_list_toggle_switches_view_mode() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("/tmp/photocraft-recent-grid/a.png");
        app.push_recent("/tmp/photocraft-recent-grid/b.psd");

        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 760.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    let recent: Vec<String> = app.ui.recent_files.clone();
                    show_home_recent(app, ui, &recent, 600.0);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.state_mut().1 = true;
        h.run_steps(2);

        // Initially in Grid mode
        assert!(h.state().0.ui.chrome.home_recent_grid);
        assert!(h.query_by_label("b.psd").is_some());
        assert!(h.query_by_label("a.png").is_some());

        // Toggle to List view
        h.get_by_label("List view").click();
        h.run_steps(2);
        assert!(!h.state().0.ui.chrome.home_recent_grid);

        // Toggle back to Grid view
        h.get_by_label("Grid view").click();
        h.run_steps(2);
        assert!(h.state().0.ui.chrome.home_recent_grid);
    }

    #[test]
    fn recent_grid_card_shows_filename_and_folder() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("/tmp/photocraft-work/document.psd");

        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 760.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    let recent: Vec<String> = app.ui.recent_files.clone();
                    show_home_recent(app, ui, &recent, 600.0);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.state_mut().1 = true;
        h.run_steps(2);

        assert!(h.query_by_label("document.psd").is_some());
    }

    #[test]
    fn recent_header_view_switch_buttons_align_to_right_edge() {
        let content_w = 640.0;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent("/tmp/photocraft-work/document.psd");

        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 760.0)).build_ui_state(
            |ui, (app, ready): &mut (PhotocraftApp, bool)| {
                if *ready {
                    let recent: Vec<String> = app.ui.recent_files.clone();
                    show_home_recent(app, ui, &recent, content_w);
                }
            },
            (app, false),
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.state_mut().1 = true;
        h.run_steps(2);

        let list_node = h.get_by_label("List view");
        let list_rect = list_node.rect();
        let box_right = list_rect.max.x + 2.0;

        let start_x = h.get_by_label("Recent").rect().min.x;
        assert_eq!(box_right, start_x + content_w, "toggle buttons must align flush to content right edge");
    }

    #[test]
    fn inline_drop_card_renders_hint_and_open_button() {
        let hint = "Drop an image or PSD anywhere to open it.";
        let mut clicked = false;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(400.0, 400.0)).build_ui(|ui| {
            if render_inline_drop_card(ui, hint) {
                clicked = true;
            }
        });
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.run_steps(2);
        assert!(h.query_by_label("Open…").is_some());
        let _ = clicked;
    }

    #[test]
    fn recent_thumbnail_cleared_on_remove_and_clear_recent() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let path1 = "/tmp/photocraft-test/thumb1.psd";
        let path2 = "/tmp/photocraft-test/thumb2.psd";

        app.push_recent(path1);
        app.push_recent(path2);

        // Pre-populate dummy entries in thumbnail_cache
        {
            let mut cache = app.thumbnail_cache.lock().unwrap();
            cache.entries.insert(
                path1.to_string(),
                ThumbnailEntry {
                    key: CacheKey::DiskFile { modified: None, len: 100 },
                    texture: None,
                    #[cfg(not(target_arch = "wasm32"))]
                    last_checked: std::time::Instant::now(),
                },
            );
            cache.entries.insert(
                path2.to_string(),
                ThumbnailEntry {
                    key: CacheKey::DiskFile { modified: None, len: 200 },
                    texture: None,
                    #[cfg(not(target_arch = "wasm32"))]
                    last_checked: std::time::Instant::now(),
                },
            );
        }

        // Verify entries exist
        assert!(app.thumbnail_cache.lock().unwrap().entries.contains_key(path1));
        assert!(app.thumbnail_cache.lock().unwrap().entries.contains_key(path2));

        // remove_recent removes from thumbnail_cache
        app.remove_recent(path1);
        assert!(!app.thumbnail_cache.lock().unwrap().entries.contains_key(path1));
        assert!(app.thumbnail_cache.lock().unwrap().entries.contains_key(path2));

        // clear_recent clears all entries from thumbnail_cache
        app.clear_recent();
        assert!(app.thumbnail_cache.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn recent_thumbnail_updates_when_open_document_revision_changes() {
        use photocraft_doc::{ColorMode, Document, SampleType, Size};

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let path = "/tmp/photocraft-test/open_doc.psd";
        app.push_recent(path);

        let doc = Document::new("open_doc.psd", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        app.session.open_document(doc, Some(path.to_string()));
        if let Some(st) = app.session.active_mut() {
            st.path = Some(path.to_string());
            st.revision = 0;
        }

        let ctx = egui::Context::default();
        let tex1 = get_or_load_thumbnail(&app, &ctx, path);
        assert!(tex1.is_some());

        // Verify cache key matches revision 0
        {
            let cache = app.thumbnail_cache.lock().unwrap();
            let entry = cache.entries.get(path).unwrap();
            assert_eq!(entry.key, CacheKey::OpenDoc { revision: 0 });
        }

        // Increment revision (simulate an edit to the open document)
        if let Some(st) = app.session.active_mut() {
            st.revision = 1;
        }

        #[cfg(not(target_arch = "wasm32"))]
        std::thread::sleep(OPEN_DOC_REGEN_INTERVAL + std::time::Duration::from_millis(20));
        let tex2 = get_or_load_thumbnail(&app, &ctx, path);
        assert!(tex2.is_some());

        // Verify cache key updated to revision 1
        {
            let cache = app.thumbnail_cache.lock().unwrap();
            let entry = cache.entries.get(path).unwrap();
            assert_eq!(entry.key, CacheKey::OpenDoc { revision: 1 });
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn open_document_thumbnail_regeneration_is_rate_limited() {
        use photocraft_doc::{ColorMode, Document, SampleType, Size};

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let path = "/tmp/photocraft-test/open_doc_throttle.psd";
        app.push_recent(path);
        let doc = Document::new("open_doc_throttle.psd", Size::new(64, 64), ColorMode::Rgb, SampleType::U8);
        app.session.open_document(doc, Some(path.to_string()));
        if let Some(st) = app.session.active_mut() {
            st.path = Some(path.to_string());
            st.revision = 0;
        }
        let ctx = egui::Context::default();
        assert!(get_or_load_thumbnail(&app, &ctx, path).is_some());

        // Rapid edits: the previous preview is served instead of recomposing every frame.
        for rev in 1..=5 {
            if let Some(st) = app.session.active_mut() {
                st.revision = rev;
            }
            assert!(get_or_load_thumbnail(&app, &ctx, path).is_some());
            let cache = app.thumbnail_cache.lock().unwrap();
            assert_eq!(cache.entries.get(path).unwrap().key, CacheKey::OpenDoc { revision: 0 });
        }

        // Once the interval has passed the latest revision is composed.
        std::thread::sleep(OPEN_DOC_REGEN_INTERVAL + std::time::Duration::from_millis(20));
        assert!(get_or_load_thumbnail(&app, &ctx, path).is_some());
        let cache = app.thumbnail_cache.lock().unwrap();
        assert_eq!(cache.entries.get(path).unwrap().key, CacheKey::OpenDoc { revision: 5 });
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_regenerates_when_disk_file_changes() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_thumb_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("test_file.png");
        let path_str = file_path.to_string_lossy().into_owned();

        // Write an initial 4x4 red PNG
        let red_png = {
            let img = photocraft_codecs::Image::from_raw(
                4,
                4,
                photocraft_codecs::ChannelLayout::Rgba,
                photocraft_codecs::SampleType::U8,
                [255, 0, 0, 255].repeat(16),
            )
            .unwrap();
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap()
        };
        std::fs::write(&file_path, &red_png).unwrap();

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent(&path_str);

        let ctx = egui::Context::default();
        let tex1 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex1.is_some());

        let initial_len = {
            let cache = app.thumbnail_cache.lock().unwrap();
            let entry = cache.entries.get(&path_str).unwrap();
            match entry.key {
                CacheKey::DiskFile { len, .. } => len,
                _ => panic!("expected DiskFile cache key"),
            }
        };

        // Write a larger 32x32 multi-color PNG to change file size and content
        let blue_png = {
            let mut px = Vec::with_capacity(32 * 32 * 4);
            for i in 0..(32 * 32) {
                px.extend_from_slice(&[(i % 256) as u8, ((i * 7) % 256) as u8, ((i * 13) % 256) as u8, 255]);
            }
            let img = photocraft_codecs::Image::from_raw(32, 32, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, px).unwrap();
            photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap()
        };
        std::fs::write(&file_path, &blue_png).unwrap();

        // Let the recheck interval elapse (10ms in tests)
        std::thread::sleep(std::time::Duration::from_millis(20));

        // get_or_load_thumbnail detects the disk metadata change and regenerates
        let tex2 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex2.is_some());

        let new_len = {
            let cache = app.thumbnail_cache.lock().unwrap();
            let entry = cache.entries.get(&path_str).unwrap();
            match entry.key {
                CacheKey::DiskFile { len, .. } => len,
                _ => panic!("expected DiskFile cache key"),
            }
        };

        assert_ne!(initial_len, new_len, "thumbnail cache must reflect the updated file size on disk");
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_recheck_interval_avoids_per_frame_disk_io() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_fast_thumb_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("test_file.png");
        let path_str = file_path.to_string_lossy().into_owned();

        let img =
            photocraft_codecs::Image::from_raw(4, 4, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, [0, 255, 0, 255].repeat(16))
                .unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(&file_path, &png).unwrap();

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.push_recent(&path_str);

        let ctx = egui::Context::default();
        let tex1 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex1.is_some());

        // Immediately remove the file from disk
        std::fs::remove_file(&file_path).unwrap();

        // Calling get_or_load_thumbnail immediately within the recheck window serves from
        // memory without touching disk or erroring out
        let tex2 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex2.is_some(), "subsequent frame within recheck interval must serve from cache");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_composite_for_oversized_psd_without_preview() {
        let temp_dir = std::env::temp_dir().join(format!("craft_psd_bound_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("oversized.psd");
        let path_str = file_path.to_string_lossy().into_owned();

        // Create an oversized PSD (3000 x 3000) with no thumbnail resource
        let header = craft_psd::Header::new(craft_psd::Version::Psd, 3000, 3000, 3, 8, craft_psd::ColorMode::Rgb);
        let file = craft_psd::PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources: Vec::new(),
            layer_info: None,
            layer_info_placement: craft_psd::LayerInfoPlacement::Section,
            global_layer_mask: None,
            global_blocks: Vec::new(),
            layer_mask_trailing: Vec::new(),
            image_data: craft_psd::ImageData { compression: craft_psd::Compression::Raw, data: Vec::new(), stored: None },
            write_as_psb: false,
        };
        let mut bytes = file.to_bytes().unwrap();
        // Append a 2 MB dummy/corrupt image payload to simulate a real multi-megabyte payload
        bytes.extend_from_slice(&vec![0xAA; 2 * 1024 * 1024]);
        std::fs::write(&file_path, &bytes).unwrap();

        // load_thumbnail_image_bounded should return None due to <= 2048 bound checked from 26-byte header,
        // without attempting to read or parse the 2 MB payload.
        let thumb = load_thumbnail_image_bounded(&path_str, bytes.len() as u64);
        assert!(thumb.is_none(), "oversized PSD with payload must skip composite fallback");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_composite_for_high_channel_psd_without_preview() {
        let temp_dir = std::env::temp_dir().join(format!("craft_psd_channels_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("high_channels.psd");
        let path_str = file_path.to_string_lossy().into_owned();

        // 2000 x 2000 with 56 channels: decoded uncompressed size = 2000 * 2000 * 56 = 224 MB (> 64 MB cap)
        let header = craft_psd::Header::new(craft_psd::Version::Psd, 2000, 2000, 56, 8, craft_psd::ColorMode::Rgb);
        let file = craft_psd::PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources: Vec::new(),
            layer_info: None,
            layer_info_placement: craft_psd::LayerInfoPlacement::Section,
            global_layer_mask: None,
            global_blocks: Vec::new(),
            layer_mask_trailing: Vec::new(),
            image_data: craft_psd::ImageData { compression: craft_psd::Compression::Raw, data: Vec::new(), stored: None },
            write_as_psb: false,
        };
        let bytes = file.to_bytes().unwrap();
        std::fs::write(&file_path, &bytes).unwrap();

        let thumb = load_thumbnail_image_bounded(&path_str, bytes.len() as u64);
        assert!(thumb.is_none(), "high-channel PSD exceeding decoded memory budget must skip composite fallback");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_extracts_embedded_preview_from_large_psd_without_reading_payload() {
        let temp_dir = std::env::temp_dir().join(format!("craft_psd_large_preview_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("large_payload.psd");
        let path_str = file_path.to_string_lossy().into_owned();

        let img = photocraft_codecs::Image::from_raw(
            512,
            512,
            photocraft_codecs::ChannelLayout::Rgb,
            photocraft_codecs::SampleType::U8,
            [255, 0, 0].repeat(512 * 512),
        )
        .unwrap();
        let jfif = photocraft_codecs::encode(&img, photocraft_codecs::Format::Jpeg, &photocraft_codecs::EncodeOptions::default()).unwrap();
        let mut res_data = vec![0u8; 28];
        res_data[0..4].copy_from_slice(&1u32.to_be_bytes()); // format: 1 (kJpegRGB)
        res_data[4..8].copy_from_slice(&512u32.to_be_bytes()); // width: 512
        res_data[8..12].copy_from_slice(&512u32.to_be_bytes()); // height: 512
        res_data.extend_from_slice(&jfif);
        let resource = craft_psd::resources::ImageResource::new(craft_psd::resources::ids::THUMBNAIL, res_data);

        let header = craft_psd::Header::new(craft_psd::Version::Psd, 400, 400, 3, 8, craft_psd::ColorMode::Rgb);
        let file = craft_psd::PsdFile {
            header,
            color_mode_data: Vec::new(),
            resources: vec![resource],
            layer_info: None,
            layer_info_placement: craft_psd::LayerInfoPlacement::Section,
            global_layer_mask: None,
            global_blocks: Vec::new(),
            layer_mask_trailing: Vec::new(),
            image_data: craft_psd::ImageData { compression: craft_psd::Compression::Raw, data: Vec::new(), stored: None },
            write_as_psb: false,
        };
        let mut bytes = file.to_bytes().unwrap();
        // Append 3 MB of corrupt payload bytes after the resources section
        bytes.extend_from_slice(&vec![0xEE; 3 * 1024 * 1024]);
        std::fs::write(&file_path, &bytes).unwrap();

        // The bounded streaming range reader must extract the embedded 1036 JFIF preview
        // from the initial resources section without erroring on or allocating the 3 MB trailing payload.
        let thumb = load_thumbnail_image_bounded(&path_str, bytes.len() as u64);
        assert!(thumb.is_some(), "must extract embedded JFIF preview despite trailing multi-megabyte payload");
        let thumb_img = thumb.unwrap();
        assert_eq!(thumb_img.size, [256, 256], "embedded previews must be downsampled before texture creation");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_loads_in_background_when_background_jobs_enabled() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_async_thumb_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("bg_test.png");
        let path_str = file_path.to_string_lossy().into_owned();

        let img =
            photocraft_codecs::Image::from_raw(8, 8, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, [0, 0, 255, 255].repeat(64))
                .unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(&file_path, &png).unwrap();

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;

        let ctx = egui::Context::default();
        // First call on cache miss: should dispatch to background worker and return None immediately (no UI stall)
        let tex1 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex1.is_none(), "cache miss with background_jobs must not block the UI thread");

        // Wait for worker thread to complete
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_secs(2) {
            let pending_done = {
                let cache = app.thumbnail_cache.lock().unwrap();
                !cache.pending.is_empty() || cache.entries.contains_key(&path_str)
            };
            if pending_done {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        // Second call: drains pending and returns the loaded texture
        let tex2 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex2.is_some(), "subsequent frame receives the loaded thumbnail from background worker");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_discards_background_result_if_invalidated_while_loading() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_inval_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("inval_test.png");
        let path_str = file_path.to_string_lossy().into_owned();

        let img =
            photocraft_codecs::Image::from_raw(8, 8, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, [255, 0, 0, 255].repeat(64))
                .unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(&file_path, &png).unwrap();

        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = true;

        let ctx = egui::Context::default();
        let tex1 = get_or_load_thumbnail(&app, &ctx, &path_str);
        assert!(tex1.is_none());

        // Immediately invalidate while thread is in flight
        app.thumbnail_cache.lock().unwrap().invalidate(&path_str);

        // Wait a short time for thread to complete
        std::thread::sleep(std::time::Duration::from_millis(50));

        // The background thread should have detected it was removed from in_flight and discarded its result
        let cache = app.thumbnail_cache.lock().unwrap();
        assert!(!cache.in_flight.contains_key(&path_str));
        assert!(cache.pending.iter().all(|(p, ..)| p != &path_str));
        assert!(!cache.entries.contains_key(&path_str));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_does_not_overwrite_active_document_in_drain() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let doc_path = "C:/fake/path/active_project.psd".to_string();

        let doc = photocraft_doc::Document::new(
            "active_project",
            photocraft_geom::Size::new(100, 100),
            photocraft_color::ColorMode::Rgb,
            photocraft_color::SampleType::U8,
        );
        let (doc_id, _) = app.session.open_document(doc, Some(doc_path.clone()));
        let st = app.session.active().unwrap();
        let revision = st.revision;

        let ctx = egui::Context::default();
        // Load initial thumbnail for open document
        let open_tex = get_or_load_thumbnail(&app, &ctx, &doc_path);
        assert!(open_tex.is_some());

        // Simulate an obsolete disk result queued in pending with active in_flight matching generation 1
        {
            let mut cache = app.thumbnail_cache.lock().unwrap();
            let disk_key = CacheKey::DiskFile { modified: None, len: 1000 };
            cache.in_flight.insert(doc_path.clone(), 1);
            cache.pending.push((doc_path.clone(), 1, disk_key, None));
        }

        // Drain pending thumbnails
        let opts = egui::TextureOptions::default();
        drain_pending_thumbnails(&app, &ctx, opts);

        // Verify the cache still holds the OpenDoc entry, NOT overwritten by disk entry
        let cache = app.thumbnail_cache.lock().unwrap();
        let entry = cache.entries.get(&doc_path).expect("entry must exist");
        assert_eq!(entry.key, CacheKey::OpenDoc { revision });

        let _ = app.session.close(0);
        let _ = doc_id;
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_superseded_generation_is_discarded() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let test_path = "C:/fake/path/image.png".to_string();
        let ctx = egui::Context::default();
        let opts = egui::TextureOptions::default();

        // Simulate request generation 1 was superseded by generation 2
        {
            let mut cache = app.thumbnail_cache.lock().unwrap();
            cache.in_flight.insert(test_path.clone(), 2);
            // Stale worker 1 pushes with req_id 1
            let disk_key = CacheKey::DiskFile { modified: None, len: 500 };
            cache.pending.push((test_path.clone(), 1, disk_key, None));
        }

        // Drain pending: request 1 should be discarded because generation 2 is active
        drain_pending_thumbnails(&app, &ctx, opts);

        let cache = app.thumbnail_cache.lock().unwrap();
        // Generation 2 must remain in flight and not be removed or overwritten by generation 1
        assert_eq!(cache.in_flight.get(&test_path).copied(), Some(2));
        assert!(!cache.entries.contains_key(&test_path));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_flat_image_larger_than_4k_dimensions() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_huge_img_test_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("huge_img.png");
        let path_str = file_path.to_string_lossy().into_owned();

        // Write a PNG with 4097x1 dimensions (exceeds 4096 cap on longest side)
        let img = photocraft_codecs::Image::from_raw(4097, 1, photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8, vec![0u8; 4097 * 4])
            .unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &photocraft_codecs::EncodeOptions::default()).unwrap();
        std::fs::write(&file_path, &png).unwrap();

        let thumb = load_thumbnail_image_bounded(&path_str, png.len() as u64);
        assert!(thumb.is_none(), "flat images exceeding 4096 on longest side must be skipped");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn recent_thumbnail_skips_oversized_thumb_in_pcraft_dir() {
        let temp_dir = std::env::temp_dir().join(format!("photocraft_oversized_pcraft_test_{}.pcraft", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_dir);
        let thumb_path = temp_dir.join("thumb.png");
        let path_str = temp_dir.to_string_lossy().into_owned();

        // Create a 65 MiB thumb.png (exceeds 64 MiB input budget) using set_len (instant sparse file)
        let f = std::fs::File::create(&thumb_path).unwrap();
        f.set_len(65 * 1024 * 1024).unwrap();
        drop(f);

        // Directory metadata len is small (typically 0 or 4096), but the inner thumb.png exceeds budget
        let dir_len = std::fs::metadata(&temp_dir).map(|m| m.len()).unwrap_or(0);
        let thumb = load_thumbnail_image_bounded(&path_str, dir_len);
        assert!(thumb.is_none(), "oversized thumb.png in .pcraft directory must be rejected before reading");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
