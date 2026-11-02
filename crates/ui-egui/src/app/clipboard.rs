use super::PhotocraftApp;

fn clip_signature(w: u32, h: u32, px: &[u8]) -> u64 {
    let mut sig = (w as u64) << 32 | h as u64;
    for b in px.iter().step_by(997) {
        sig = sig.rotate_left(5) ^ *b as u64;
    }
    sig
}

/// Match the bitmap signature by sampling the bounded SVG mirror, without another full pixel buffer.
fn svg_clip_signature(clip: &photocraft_engine::edit_cmds::Clip) -> u64 {
    let b = clip.bounds;
    let (w, h) = (b.width(), b.height());
    let mut sig = (u64::from(w) << 32) | u64::from(h);
    if w == 0 || h == 0 {
        return sig;
    }
    for i in (0..b.size().area().saturating_mul(4)).step_by(997) {
        let x = b.x0.saturating_add((i / 4 % u64::from(w)) as i32);
        let y = b.y0.saturating_add((i / 4 / u64::from(w)) as i32);
        let mut pixel = [[0; 4]];
        clip.surface.read_rgba8_into(photocraft_geom::Rect::from_xywh(x, y, 1, 1), &mut pixel);
        let value = pixel[0].get((i % 4) as usize).copied().unwrap_or_default();
        sig = sig.rotate_left(5) ^ u64::from(value);
    }
    sig
}

impl PhotocraftApp {
    /// Capture SVG and its raster mirror together; failures leave the previous copy intact.
    pub(super) fn import_os_svg_clipboard(&mut self) -> Result<bool, String> {
        let Some(get) = self.services.clipboard_get_svg.as_mut() else { return Ok(false) };
        let source = match get() {
            Ok(source) => source,
            Err(_) if self.clip_unmirrored && !self.clip_external && self.session.clipboard.is_some() => return Ok(false),
            Err(error) => {
                self.ui.status = error.clone();
                self.ui.status_error = true;
                return Err(error);
            }
        };
        let Some(source) = source else {
            self.os_clip_svg = None;
            // Bitmap equality cannot prove that an earlier external vector source is current.
            if self.clip_external && self.session.clipboard.as_ref().is_some_and(|clip| clip.svg.is_some()) {
                self.session.clipboard = None;
                self.os_clip_sig = None;
                self.clip_external = false;
                self.clip_unmirrored = true;
            }
            return Ok(false);
        };
        if self.os_clip_svg.as_deref().is_some_and(|old| old == &source) && self.session.clipboard.is_some() {
            // A newer internal copy remains authoritative until the external bytes change.
            return Ok(true);
        }
        let clip = photocraft_engine::edit_cmds::Clip::from_svg(&source).map_err(|error| {
            let error = error.to_string();
            self.ui.status = error.clone();
            self.ui.status_error = true;
            error
        })?;
        self.os_clip_sig = Some(svg_clip_signature(&clip));
        self.os_clip_svg = clip.svg.clone();
        self.session.clipboard = Some(clip);
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }

    /// Mirror the session clipboard onto the OS clipboard (RGBA8).
    pub(super) fn export_os_clipboard(&mut self) {
        self.clip_unmirrored = true;
        if !self.session.prefs().general.export_clipboard {
            return;
        }
        let (Some(set), Some(clip)) = (self.services.clipboard_set_image.as_mut(), self.session.clipboard.as_ref()) else { return };
        let b = clip.bounds;
        if b.is_empty() {
            return;
        }
        let mut px = vec![[0u8; 4]; b.width() as usize * b.height() as usize];
        clip.surface.read_rgba8_into(b, &mut px);
        let bytes: Vec<u8> = px.into_iter().flatten().collect();
        if set(b.width(), b.height(), &bytes).is_ok() {
            self.os_clip_svg = None;
            self.os_clip_sig = Some(clip_signature(b.width(), b.height(), &bytes));
            self.clip_unmirrored = false;
        }
    }

    /// If the OS clipboard holds an image that isn't the one we put there, make it the session
    /// clipboard (so ⌘V pastes screenshots and images copied in other apps, like Photoshop).
    /// Returns `Ok(true)` when a new external image was imported. Once the OS clipboard no longer holds
    /// the image we mirrored or imported (text or a file was copied since), that image is stale:
    /// it is dropped rather than pasted. Unexported internal copies survive unavailable or empty
    /// system clipboards.
    pub(crate) fn import_os_clipboard(&mut self) -> Result<bool, String> {
        let Some(get) = self.services.clipboard_get_image.as_mut() else { return Ok(false) };
        // A read failure says nothing about the clipboard contents. Keep our editable copy and
        // its signature rather than treating the failure as an empty image.
        let image = match get() {
            Ok(image) => image,
            Err(error) => {
                // An unexported copy remains usable without an OS clipboard (for example, in
                // a Wayland sandbox). Mirrored/imported copies still require a successful read
                // before pasting, because their system contents may have changed.
                if self.clip_unmirrored && !self.clip_external && self.session.clipboard.is_some() {
                    return Ok(false);
                }
                if self.session.clipboard.is_none() && self.services.clipboard_unreadable.as_ref().is_some_and(|unreadable| unreadable()) {
                    // Let paste explain the unavailable backend without treating a saved copy as empty.
                    return Ok(false);
                }
                self.ui.status = error.clone();
                self.ui.status_error = true;
                return Err(error);
            }
        };
        let image = image.filter(|(w, h, bytes)| *w > 0 && *h > 0 && bytes.len() == *w as usize * *h as usize * 4);
        let Some((w, h, bytes)) = image else {
            if self.clip_unmirrored && !self.clip_external && self.session.clipboard.is_some() {
                self.os_clip_sig = None;
                return Ok(false);
            }
            self.os_clip_svg = None;
            if (self.os_clip_sig.take().is_some() || self.clip_external) && !self.clip_unmirrored {
                self.session.clipboard = None;
                self.clip_external = false;
                self.clip_unmirrored = true;
            }
            return Ok(false);
        };
        let sig = clip_signature(w, h, &bytes);
        if self.os_clip_sig == Some(sig) && self.session.clipboard.is_some() {
            return Ok(false);
        }
        let r = photocraft_geom::Rect::new(0, 0, w as i32, h as i32);
        let mut surface = photocraft_raster::Surface::from_interleaved(photocraft_color::PixelFormat::RGBA8, r, &bytes);
        surface.prune();
        self.session.clipboard =
            Some(photocraft_engine::edit_cmds::Clip { components: None, surface, bounds: r, layers: None, resolution: None, source: None, svg: None });
        self.os_clip_svg = None;
        self.os_clip_sig = Some(sig);
        self.clip_external = true;
        self.clip_unmirrored = false;
        Ok(true)
    }
}
