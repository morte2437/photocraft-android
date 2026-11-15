//! Camera-field EXIF compatibility facade. Parsing, bounds and TIFF serialization live in
//! craft-metadata; the raw-date and numeric selection semantics are preserved there.

pub use craft_metadata::camera_fields::{CameraInfo, read, try_build};

/// Build a TIFF-structured EXIF payload, preserving the existing infallible API.
/// Returns an empty payload when the shared writer refuses an oversized value. New callers
/// that need the error should use [`try_build`].
pub fn build(info: &CameraInfo) -> Vec<u8> {
    try_build(info).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_survives_garbage() {
        let info = CameraInfo {
            make: Some("Generic".into()),
            model: Some("Camera 1".into()),
            lens: Some("24-70mm".into()),
            exposure_time: Some(0.004),
            f_number: Some(8.0),
            iso: Some(200.0),
            focal_length: Some(35.0),
            focal_length_35mm: Some(52.0),
            date_taken: Some("2024:02:17 12:40:19".into()),
        };
        let b = build(&info);
        let r = read(&b);
        assert_eq!(r, info);
        assert!((r.exposure_factor().unwrap() - 0.004 * 2.0 / 64.0).abs() < 1e-9);
        let mut prefixed = b"Exif\0\0".to_vec();
        prefixed.extend_from_slice(&b);
        assert_eq!(read(&prefixed), info);
        assert_eq!(read(b"nonsense"), CameraInfo::default());
        for cut in [5, 12, 30, b.len() - 3] {
            let _ = read(&b[..cut]);
        }
    }
}
