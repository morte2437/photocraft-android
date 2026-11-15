// Frozen differential oracle: PhotoCraft main abb30dff, exif.rs. Test-only, not a production fallback.
#![allow(dead_code)]
// Minimal EXIF reader for the camera settings the photography commands use (exposure, aperture,
// ISO, focal length, camera and lens names). Input is the raw TIFF-structured EXIF payload
// (`II*\0` / `MM\0*`, as stored in `Document.metadata.exif`); malformed data yields `None`
// fields, never a panic. Tag numbers are from the EXIF 2.32 specification (CIPA DC-008).

/// Camera settings read from EXIF.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraInfo {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    /// Seconds.
    pub exposure_time: Option<f64>,
    pub f_number: Option<f64>,
    pub iso: Option<f64>,
    /// Millimetres.
    pub focal_length: Option<f64>,
    /// 35 mm-equivalent focal length in millimetres.
    pub focal_length_35mm: Option<f64>,
    /// DateTimeOriginal as stored (`YYYY:MM:DD HH:MM:SS`).
    pub date_taken: Option<String>,
}

impl CameraInfo {
    /// Linear exposure factor `t · ISO / N²` (relative brightness of the image for one scene).
    pub fn exposure_factor(&self) -> Option<f64> {
        let t = self.exposure_time?;
        let n = self.f_number.unwrap_or(1.0).max(0.1);
        Some(t * self.iso.unwrap_or(100.0) / 100.0 / (n * n))
    }
}

/// Text bytes decoded per EXIF read (real Make/Model/Lens/Date strings are a few dozen bytes).
const MAX_TEXT_BYTES: usize = 1 << 20;

struct Tiff<'a> {
    b: &'a [u8],
    le: bool,
    /// Text bytes this read may still decode: duplicate tags can share one large blank value.
    text_left: std::cell::Cell<usize>,
}

impl Tiff<'_> {
    fn u16(&self, o: usize) -> Option<u16> {
        let s = self.b.get(o..o.checked_add(2)?)?;
        Some(if self.le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
    }
    fn u32(&self, o: usize) -> Option<u32> {
        let s = self.b.get(o..o.checked_add(4)?)?;
        let a = [s[0], s[1], s[2], s[3]];
        Some(if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    }
    /// Entries of the IFD at `off`: (tag, type, count, value offset field position).
    fn ifd(&self, off: usize) -> Vec<(u16, u16, u32, usize)> {
        let Some(n) = self.u16(off) else { return Vec::new() };
        (0..n as usize)
            .filter_map(|k| {
                // Checked: offsets come from the file, and usize is 32 bits on wasm.
                let e = off.checked_add(2)?.checked_add(k * 12)?;
                Some((self.u16(e)?, self.u16(e.checked_add(2)?)?, self.u32(e.checked_add(4)?)?, e.checked_add(8)?))
            })
            .collect()
    }
    fn data_at(&self, typ: u16, count: u32, field: usize) -> Option<usize> {
        let size = match typ {
            1 | 2 | 6 | 7 => 1usize,
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => return None,
        }
        .checked_mul(usize::try_from(count).ok()?)?;
        let at = if size <= 4 { field } else { usize::try_from(self.u32(field)?).ok()? };
        // The whole value must lie inside the payload, not just its first element.
        self.b.get(at..at.checked_add(size)?)?;
        Some(at)
    }
    fn number(&self, typ: u16, count: u32, field: usize) -> Option<f64> {
        let at = self.data_at(typ, count, field)?;
        match typ {
            3 => self.u16(at).map(f64::from),
            4 => self.u32(at).map(f64::from),
            5 => {
                let (n, d) = (self.u32(at)?, self.u32(at.checked_add(4)?)?);
                (d != 0).then(|| n as f64 / d as f64)
            }
            10 => {
                let (n, d) = (self.u32(at)? as i32, self.u32(at.checked_add(4)?)? as i32);
                (d != 0).then(|| n as f64 / d as f64)
            }
            _ => None,
        }
    }
    fn text(&self, typ: u16, count: u32, field: usize) -> Option<String> {
        #[cfg(test)]
        TEXT_DECODES.with(|c| c.set(c.get() + 1));
        if typ != 2 {
            return None;
        }
        let at = self.data_at(typ, count, field)?;
        let n = usize::try_from(count).ok()?;
        self.text_left.set(self.text_left.get().checked_sub(n)?);
        let s = self.b.get(at..at.checked_add(n)?)?;
        let s = String::from_utf8_lossy(s).trim_end_matches('\0').trim().to_string();
        (!s.is_empty()).then_some(s)
    }
}

#[cfg(test)]
thread_local!(static TEXT_DECODES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) });

/// Reads [`CameraInfo`] from a TIFF-structured EXIF payload (an `Exif\0\0` prefix is skipped).
pub fn read(exif: &[u8]) -> CameraInfo {
    let b = exif.strip_prefix(b"Exif\0\0").unwrap_or(exif);
    let mut info = CameraInfo::default();
    let le = match b.get(..4) {
        Some([b'I', b'I', 42, 0]) => true,
        Some([b'M', b'M', 0, 42]) => false,
        _ => return info,
    };
    let t = Tiff { b, le, text_left: std::cell::Cell::new(MAX_TEXT_BYTES) };
    let Some(ifd0) = t.u32(4) else { return info };
    let mut queue = vec![ifd0 as usize];
    let mut seen = Vec::new();
    while let Some(off) = queue.pop() {
        if seen.contains(&off) || seen.len() > 8 {
            continue;
        }
        seen.push(off);
        for (tag, typ, count, field) in t.ifd(off) {
            match tag {
                // Duplicate TIFF text tags can point at the same large payload. Keep the first
                // recognized value and do not repeatedly decode/copy later duplicates.
                0x010F if info.make.is_none() => info.make = t.text(typ, count, field),
                0x0110 if info.model.is_none() => info.model = t.text(typ, count, field),
                0x829A => info.exposure_time = t.number(typ, count, field),
                0x829D => info.f_number = t.number(typ, count, field),
                0x8827 => info.iso = t.number(typ, count, field),
                0x920A => info.focal_length = t.number(typ, count, field),
                0xA405 => info.focal_length_35mm = t.number(typ, count, field).filter(|v| *v > 0.0),
                0xA434 if info.lens.is_none() => info.lens = t.text(typ, count, field),
                0x9003 if info.date_taken.is_none() => info.date_taken = t.text(typ, count, field),
                0x8769 => {
                    if let Some(p) = t.u32(field) {
                        queue.push(p as usize);
                    }
                }
                _ => {}
            }
        }
    }
    info
}
